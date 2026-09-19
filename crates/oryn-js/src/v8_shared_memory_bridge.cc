// Maps broker-owned shared memory into the actual V8 sandbox address space.
// The descriptor is borrowed for this call; the backing store owns its mapping.
#include "v8-array-buffer.h"
#include "v8-isolate.h"
#include "v8-platform.h"
#include <atomic>
#include <cstdint>
#include <memory>
#include <new>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
#if defined(__APPLE__)
#include <mach/mach.h>
#include <mach/mach_vm.h>
#endif

namespace {
using GrowCallback = int (*)(void*, size_t);
using GrowPagesCallback = bool (*)(void*, size_t, size_t, size_t*);
using ReleaseCallback = void (*)(void*);
// Ownership crosses the FFI boundary at entry, including every failure path.
struct CallbackLease {
  void* context;
  GrowCallback grow;
  GrowPagesCallback grow_pages;
  ReleaseCallback release;
  ~CallbackLease() { if (release) release(context); }
};
struct Mapping {
#ifdef V8_ENABLE_SANDBOX
  v8::IsolateGroup group;
  v8::VirtualAddressSpace* space;
  Mapping(v8::IsolateGroup group, v8::VirtualAddressSpace* space,
          uintptr_t address, size_t size)
      : group(std::move(group)), space(space), address(address), size(size) {}
#else
  Mapping(uintptr_t address, size_t size) : address(address), size(size) {}
#endif
  std::shared_ptr<CallbackLease> lease;
  uintptr_t address;
  size_t size;
  void* metadata = MAP_FAILED;
  size_t metadata_size = 0;
  ~Mapping() {
    if (metadata != MAP_FAILED) munmap(metadata, metadata_size);
#ifdef V8_ENABLE_SANDBOX
    space->FreeSharedPages(address, size);
#else
    munmap(reinterpret_cast<void*>(address), size);
#endif
  }
};
void DeleteMapping(void*, size_t, void* owner) {
  delete static_cast<std::shared_ptr<Mapping>*>(owner);
}
class GrowthDelegate final : public v8::SharedMemoryGrowthDelegate {
 public:
  explicit GrowthDelegate(std::shared_ptr<Mapping> mapping)
      : mapping_(std::move(mapping)) {}
  size_t ByteLength() const override {
    // The coordinator publishes a naturally aligned native u64 atomic at fd 0.
    // The mapping is shared across processes; no V8-local length cache is used.
    return static_cast<size_t>(__atomic_load_n(
        static_cast<const uint64_t*>(mapping_->metadata), __ATOMIC_ACQUIRE));
  }
  const size_t* ByteLengthAddress() const override {
    // The qualified native shared-memory profile is 64-bit, matching the
    // coordinator's u64 atomic metadata cell without truncation or aliasing.
    static_assert(sizeof(size_t) == sizeof(uint64_t));
    return static_cast<const size_t*>(mapping_->metadata);
  }
  bool GrowWasmByPages(size_t delta, size_t maximum, size_t* previous) override {
    const auto& lease = mapping_->lease;
    return lease && lease->grow_pages &&
        lease->grow_pages(lease->context, delta, maximum, previous);
  }
  Result GrowAbsolute(size_t length) override {
    const auto& lease = mapping_->lease;
    if (!lease || !lease->grow) return Result::kFailure;
    switch (lease->grow(lease->context, length)) {
      case 0: return Result::kSuccess;
      case 2: return Result::kRace;
      default: return Result::kFailure;
    }
  }
 private:
  std::shared_ptr<Mapping> mapping_;
};
}

extern "C" size_t oryn_v8_shared_memory_granularity(v8::Isolate* isolate) {
#ifdef V8_ENABLE_SANDBOX
  auto group = isolate->GetGroup();
  auto* space = group.GetSandboxAddressSpace();
  return space ? space->allocation_granularity() : 0;
#else
  const long page = sysconf(_SC_PAGESIZE);
  return page > 0 ? static_cast<size_t>(page) : 0;
#endif
}

static const v8::SharedArrayBuffer* MapSharedMemory(
    v8::Isolate* isolate, int fd, size_t byte_length, size_t mapped_length,
    size_t data_offset, size_t reserved_length, size_t guard_length,
    size_t maximum_length, bool growable, GrowCallback grow,
    ReleaseCallback release, void* context, bool wasm, bool memory64,
    GrowPagesCallback grow_pages) {
  auto lease = std::make_shared<CallbackLease>(CallbackLease{context, grow, grow_pages, nullptr});
  lease->release = release;
  const size_t granularity = oryn_v8_shared_memory_granularity(isolate);
  struct stat status;
  if (!granularity || fd < 0 || !mapped_length ||
      mapped_length % granularity || data_offset < sizeof(uint64_t) ||
      data_offset % granularity || reserved_length % granularity ||
      data_offset > mapped_length || reserved_length != mapped_length - data_offset ||
      byte_length > maximum_length || maximum_length > reserved_length ||
      (!wasm && !growable && maximum_length != byte_length) ||
      (!wasm && growable && (!grow || !release)) ||
      (wasm && (!grow_pages || !release || byte_length % 65536 || maximum_length % 65536)) || guard_length != 0 ||
      mapped_length > 256u * 1024u * 1024u || fstat(fd, &status) != 0 ||
      status.st_size < 0 || static_cast<uint64_t>(status.st_size) != mapped_length)
    return nullptr;
  uintptr_t address = 0;
#ifdef V8_ENABLE_SANDBOX
  auto group = isolate->GetGroup();
  auto* space = group.GetSandboxAddressSpace();
  if (!space) return nullptr;
#if defined(__APPLE__)
  // V8's Darwin virtual address allocator requires a Mach memory-entry right.
  // Derive it from this received descriptor's actual shared mapping, never an
  // integer from a serialized message or an author-selected Mach port name.
  void* temporary = mmap(nullptr, reserved_length, PROT_READ | PROT_WRITE,
                         MAP_SHARED, fd, data_offset);
  if (temporary == MAP_FAILED) return nullptr;
  mach_port_t memory = MACH_PORT_NULL;
  memory_object_size_t extent = reserved_length;
  const auto result = mach_make_memory_entry_64(
      mach_task_self(), &extent, reinterpret_cast<memory_object_offset_t>(temporary),
      VM_PROT_READ | VM_PROT_WRITE, &memory, MACH_PORT_NULL);
  if (result == KERN_SUCCESS && extent >= reserved_length) {
    address = space->AllocateSharedPages(
        0, reserved_length, v8::PagePermissions::kReadWrite,
        v8::SharedMemoryHandle::FromPlatformHandle(memory), 0);
  }
  if (memory != MACH_PORT_NULL) mach_port_deallocate(mach_task_self(), memory);
  munmap(temporary, reserved_length);
#else
  address = space->AllocateSharedPages(
      0, reserved_length, v8::PagePermissions::kReadWrite,
      v8::SharedMemoryHandle::FromPlatformHandle(fd), data_offset);
#endif
  if (!address) return nullptr;
  auto* owner = new (std::nothrow) Mapping(std::move(group), space, address, reserved_length);
  if (!owner) { space->FreeSharedPages(address, reserved_length); return nullptr; }
#else
  void* mapping = mmap(nullptr, reserved_length, PROT_READ | PROT_WRITE, MAP_SHARED, fd, data_offset);
  if (mapping == MAP_FAILED) return nullptr;
  address = reinterpret_cast<uintptr_t>(mapping);
  auto* owner = new (std::nothrow) Mapping(address, reserved_length);
  if (!owner) { munmap(mapping, reserved_length); return nullptr; }
#endif
  std::shared_ptr<Mapping> mapping_owner(owner);
  owner->lease = std::move(lease);
  owner->metadata = mmap(nullptr, data_offset, PROT_READ, MAP_SHARED, fd, 0);
  owner->metadata_size = data_offset;
  if (owner->metadata == MAP_FAILED) return nullptr;
  const uint64_t current = __atomic_load_n(
      static_cast<const uint64_t*>(owner->metadata), __ATOMIC_ACQUIRE);
  if (current < byte_length || current > maximum_length ||
      (!wasm && !growable && current != byte_length)) return nullptr;
  auto* backing_owner = new (std::nothrow) std::shared_ptr<Mapping>(mapping_owner);
  if (!backing_owner) return nullptr;
  std::unique_ptr<v8::BackingStore> backing;
  if (wasm) {
    // The Wasm factory consumes backing_owner even when validation fails.
    backing = v8::BackingStore::NewSharedWasmBackingStore(
        reinterpret_cast<void*>(address), byte_length, maximum_length,
        reserved_length, memory64, std::make_shared<GrowthDelegate>(mapping_owner),
        DeleteMapping, backing_owner);
    if (!backing) return nullptr;
  } else {
    backing = v8::SharedArrayBuffer::NewBackingStore(
        reinterpret_cast<void*>(address), byte_length, DeleteMapping, backing_owner);
    if (!backing) { delete backing_owner; return nullptr; }
  }
  if (!wasm && growable) {
    backing->SetSharedMemoryGrowthDelegate(
        std::make_shared<GrowthDelegate>(mapping_owner), maximum_length,
        reserved_length);
  }
  return *v8::SharedArrayBuffer::New(isolate, std::move(backing));
}


extern "C" const v8::SharedArrayBuffer* oryn_v8_map_shared_memory(
    v8::Isolate* isolate, int fd, size_t byte_length, size_t mapped_length,
    size_t data_offset, size_t reserved_length, size_t guard_length,
    size_t maximum_length, bool growable, GrowCallback grow,
    ReleaseCallback release, void* context) {
  return MapSharedMemory(isolate, fd, byte_length, mapped_length, data_offset,
      reserved_length, guard_length, maximum_length, growable, grow, release,
      context, false, false, nullptr);
}

// The returned temporary SAB is a native handle to the Wasm backing. The caller
// must attach that backing to a real WasmMemoryObject through the engine factory;
// it must not substitute this wrapper for a WebAssembly.Memory instance.
extern "C" const v8::SharedArrayBuffer* oryn_v8_map_shared_wasm_memory(
    v8::Isolate* isolate, int fd, size_t byte_length, size_t mapped_length,
    size_t data_offset, size_t reserved_length, size_t guard_length,
    size_t maximum_length, bool memory64, GrowPagesCallback grow_pages,
    ReleaseCallback release, void* context) {
  return MapSharedMemory(isolate, fd, byte_length, mapped_length, data_offset,
      reserved_length, guard_length, maximum_length, false, nullptr, release,
      context, true, memory64, grow_pages);
}

// The callback receives a rooted current context, never an author-supplied
// principal. Rust copies a backing owner into the out slot while its callback
// handle scope is alive; no Local handle crosses the callback's destruction.
extern "C" bool oryn_v8_allocate_native_shared_memory(
    v8::Local<v8::Context> context, size_t initial, size_t maximum,
    bool growable, bool wasm, bool memory64, void* backing_out);
namespace {
struct NativeAllocationDelegate final : v8::SharedMemoryAllocationDelegate {
  std::shared_ptr<v8::BackingStore> AllocateSharedMemory(
      v8::Isolate* isolate, size_t initial, size_t maximum,
      bool growable, bool wasm, bool memory64) override {
    auto context = isolate->GetCurrentContext();
    if (context.IsEmpty()) return {};
    std::shared_ptr<v8::BackingStore> backing;
    if (!oryn_v8_allocate_native_shared_memory(
            context, initial, maximum, growable, wasm, memory64, &backing))
      return {};
    return backing;
  }
};
}
extern "C" void oryn_v8_install_shared_memory_factory(v8::Isolate* isolate) {
  isolate->SetSharedMemoryAllocationDelegate(
      std::make_shared<NativeAllocationDelegate>());
}
extern "C" bool oryn_v8_copy_shared_factory_backing(
    const v8::SharedArrayBuffer* buffer, void* backing_out) {
  if (!buffer || !backing_out) return false;
  auto backing = const_cast<v8::SharedArrayBuffer*>(buffer)->GetBackingStore();
  if (!backing) return false;
  *static_cast<std::shared_ptr<v8::BackingStore>*>(backing_out) = std::move(backing);
  return true;
}
