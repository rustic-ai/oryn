// Native engine regression source. Run only after the global generation gate.
#include "test/cctest/cctest.h"
#include "include/v8-array-buffer.h"
#include "src/execution/futex-emulation.h"
#include "src/objects/backing-store.h"
#include <atomic>
#include <cstdlib>
#include "include/v8-value-serializer.h"
#include <memory>
#include <thread>

namespace {
struct SharedLength final : v8::SharedMemoryGrowthDelegate {
  std::atomic<size_t> length{16};
  bool fail = false;
  size_t ByteLength() const override { return length.load(std::memory_order_acquire); }
  const size_t* ByteLengthAddress() const override {
    return reinterpret_cast<const size_t*>(&length);
  }
  Result GrowAbsolute(size_t next) override {
    if (fail || next > 4096) return Result::kFailure;
    size_t previous = length.load(std::memory_order_acquire);
    for (;;) {
      if (next < previous) return Result::kRace;
      if (length.compare_exchange_weak(previous, next, std::memory_order_acq_rel))
        return Result::kSuccess;
    }
  }
};
struct Allocation {
  std::unique_ptr<v8::BackingStore> storage;
  std::shared_ptr<std::atomic<int>> releases;
  ~Allocation() { ++*releases; }
};
void Release(void*, size_t, void* allocation) {
  delete static_cast<Allocation*>(allocation);
}
std::unique_ptr<v8::BackingStore> MakeBacking(
    v8::Isolate* isolate, std::shared_ptr<SharedLength> length, std::shared_ptr<std::atomic<int>> releases, size_t maximum = 4096) {
  // Use V8's own allocation so the pointer belongs to its actual sandbox.
  auto storage = v8::SharedArrayBuffer::NewBackingStore(isolate, 4096);
  void* data = storage->Data();
  auto backing = v8::SharedArrayBuffer::NewBackingStore(
      data, 16, Release, new Allocation{std::move(storage), releases});
  backing->SetSharedMemoryGrowthDelegate(std::move(length), maximum, 4096);
  return backing;
}
}

TEST(OrynSharedGrowthLengthAndViews) {
  LocalContext context;
  auto* isolate = context.isolate();
  v8::HandleScope handles(isolate);
  auto releases = std::make_shared<std::atomic<int>>(0);
  auto length = std::make_shared<SharedLength>();
  auto backing = MakeBacking(isolate, length, releases);
  auto buffer = v8::SharedArrayBuffer::New(isolate, std::move(backing));
  CHECK(context->Global()->Set(context.local(), v8_str("shared"), buffer).FromJust());
  CHECK(CompileRun("globalThis.tracking = new Uint8Array(shared);"
                   "globalThis.fixed = new Uint8Array(shared, 0, 16);"
                   "tracking[0] = 37; shared.grow(64);"
                   "shared.growable && shared.maxByteLength === 4096 &&"
                   "shared.byteLength === 64 && tracking.length === 64 &&"
                   "fixed.length === 16 && tracking[0] === 37 && tracking[63] === 0")
            ->IsTrue());
  // Simulates the metadata publication made by a distinct native mapping.
  length->length.store(128, std::memory_order_release);
  CHECK(CompileRun("shared.byteLength === 128 && tracking.length === 128 &&"
                   "fixed.length === 16 && tracking[127] === 0")->IsTrue());
  CHECK(CompileRun("(() => { try { shared.grow(32); return false; }"
                   "catch (e) { return e instanceof RangeError; } })()")
            ->IsTrue());
  length->fail = true;
  CHECK(CompileRun("(() => { try { shared.grow(256); return false; }"
                   "catch (e) { return e instanceof RangeError && shared.byteLength === 128; } })()")
            ->IsTrue());
  // Allocation retains the shared release counter even if GC occurs later.
}

TEST(OrynSharedGrowthCustomDeleterExactlyOnce) {
  LocalContext context;
  auto* isolate = context.isolate();
  auto releases = std::make_shared<std::atomic<int>>(0);
  {
    auto length = std::make_shared<SharedLength>();
    auto backing = MakeBacking(isolate, length, releases);
    CHECK_EQ(0, releases->load());
    CHECK_EQ(16u, backing->ByteLength());
    length->length.store(32, std::memory_order_release);
    CHECK_EQ(32u, backing->ByteLength());
  }
  CHECK_EQ(1, releases->load());
}

TEST(OrynSharedGrowthMaximumEqualsInitial) {
  LocalContext context;
  auto* isolate = context.isolate();
  v8::HandleScope handles(isolate);
  auto releases = std::make_shared<std::atomic<int>>(0);
  auto length = std::make_shared<SharedLength>();
  auto backing = MakeBacking(isolate, length, releases, 16);
  auto buffer = v8::SharedArrayBuffer::New(isolate, std::move(backing));
  CHECK(context->Global()->Set(context.local(), v8_str("shared"), buffer).FromJust());
  CHECK(CompileRun("shared.grow(16); shared.growable && "
                   "shared.maxByteLength === 16 && shared.byteLength === 16")
            ->IsTrue());
  CHECK(CompileRun("(() => { try { shared.grow(17); return false; }"
                   "catch (e) { return e instanceof RangeError; } })()")
            ->IsTrue());
}

namespace {
struct WaitProbe {
  v8::SharedMemoryWaiter::State outcome = v8::SharedMemoryWaiter::State::kNotEqual;
  size_t offset = 0;
  uint64_t expected = 0;
  int64_t timeout = 0;
  bool width64 = false;
  bool terminate = false;
  int retired = 0;
  int notified = 0;
};
class ProbeWaiter final : public v8::SharedMemoryWaiter {
 public:
  ProbeWaiter(v8::Isolate* isolate, std::shared_ptr<WaitProbe> probe)
      : isolate_(isolate), probe_(std::move(probe)) {}
  ~ProbeWaiter() override { ++probe_->retired; }
  State Poll(uint32_t maximum) override {
    CHECK_LE(maximum, 50u);
    if (probe_->terminate) {
      isolate_->TerminateExecution();
      return State::kAdmitting;
    }
    return probe_->outcome;
  }
 private:
  v8::Isolate* isolate_;
  std::shared_ptr<WaitProbe> probe_;
};
class ProbeWaitDelegate final : public v8::SharedMemoryWaitDelegate {
 public:
  ProbeWaitDelegate(v8::Isolate* isolate, std::shared_ptr<WaitProbe> probe)
      : isolate_(isolate), probe_(std::move(probe)) {}
  std::unique_ptr<v8::SharedMemoryWaiter> Begin(size_t offset, bool width64,
      uint64_t expected, int64_t timeout) override {
    probe_->offset = offset;
    probe_->width64 = width64;
    probe_->expected = expected;
    probe_->timeout = timeout;
    return std::make_unique<ProbeWaiter>(isolate_, probe_);
  }
  v8::MaybeLocal<v8::Object> WaitAsync(v8::Isolate* isolate, size_t offset,
      bool width64, uint64_t expected, int64_t timeout,
      ProcessInterrupts process_interrupts, void* interrupt_context) override {
    auto waiter = Begin(offset, width64, expected, timeout);
    while (probe_->terminate) {
      if (!process_interrupts(interrupt_context)) return {};
      CHECK(waiter->Poll(50) == v8::SharedMemoryWaiter::State::kAdmitting);
    }
    auto result = v8::Object::New(isolate);
    auto context = isolate->GetCurrentContext();
    CHECK(result->Set(context, v8_str("async"), v8::False(isolate)).FromJust());
    CHECK(result->Set(context, v8_str("value"), v8_str("not-equal")).FromJust());
    return result;
  }
  uint32_t Notify(size_t offset, uint32_t count) override {
    probe_->offset = offset;
    ++probe_->notified;
    return count == 0 ? 0 : 1;
  }
 private:
  v8::Isolate* isolate_;
  std::shared_ptr<WaitProbe> probe_;
};
void InstallWaitProbe(LocalContext& context, std::shared_ptr<WaitProbe> probe) {
  auto* isolate = context.isolate();
  isolate->SetAllowAtomicsWait(true);
  auto backing = MakeBacking(isolate, std::make_shared<SharedLength>(),
      std::make_shared<std::atomic<int>>(0));
  backing->SetSharedMemoryWaitDelegate(std::make_shared<ProbeWaitDelegate>(isolate, probe));
  auto buffer = v8::SharedArrayBuffer::New(isolate, std::move(backing));
  CHECK(context->Global()->Set(context.local(), v8_str("shared"), buffer).FromJust());
}
}

TEST(OrynNativeWaitDispatchAndBitPatterns) {
  LocalContext context;
  auto* isolate = context.isolate();
  v8::HandleScope handles(isolate);
  auto probe = std::make_shared<WaitProbe>();
  InstallWaitProbe(context, probe);
  CHECK(CompileRun("Atomics.wait(new Int32Array(shared), 1, -1, 0) === 'not-equal'")->IsTrue());
  CHECK_EQ(4u, probe->offset);
  CHECK_EQ(uint64_t{0xffffffff}, probe->expected);
  CHECK_EQ(0, probe->timeout);
  CHECK(!probe->width64);
  CHECK_EQ(1, probe->retired);
  CHECK(CompileRun("Atomics.wait(new BigInt64Array(shared), 1, -1n) === 'not-equal'")->IsTrue());
  CHECK_EQ(8u, probe->offset);
  CHECK_EQ(~uint64_t{0}, probe->expected);
  CHECK_EQ(-1, probe->timeout);
  CHECK(probe->width64);
  CHECK_EQ(2, probe->retired);
  probe->outcome = v8::SharedMemoryWaiter::State::kTimedOut;
  CHECK(CompileRun("Atomics.wait(new Int32Array(shared), 0, 0, 0) === 'timed-out'")->IsTrue());
  CHECK(CompileRun("Atomics.notify(new Int32Array(shared), 2, 3) === 1")->IsTrue());
  CHECK_EQ(8u, probe->offset);
  CHECK_EQ(1, probe->notified);
  CHECK(CompileRun("(() => {const r=Atomics.waitAsync(new Int32Array(shared),0,1,0);"
                   "return r.async === false && r.value === 'not-equal';})()")->IsTrue());
}

TEST(OrynNativeWaitTerminationReclaimsWaiter) {
  LocalContext context;
  auto* isolate = context.isolate();
  v8::HandleScope handles(isolate);
  auto probe = std::make_shared<WaitProbe>();
  probe->terminate = true;
  InstallWaitProbe(context, probe);
  {
    v8::TryCatch caught(isolate);
    auto script = v8::Script::Compile(context.local(),
        v8_str("Atomics.wait(new Int32Array(shared),0,0)")).ToLocalChecked();
    CHECK(script->Run(context.local()).IsEmpty());
    CHECK(caught.HasTerminated());
  }
  isolate->CancelTerminateExecution();
  CHECK_EQ(1, probe->retired);
}

TEST(OrynNativeWaitAsyncAdmissionTerminationReclaimsWaiter) {
  LocalContext context;
  auto* isolate = context.isolate();
  v8::HandleScope handles(isolate);
  auto probe = std::make_shared<WaitProbe>();
  probe->terminate = true;
  InstallWaitProbe(context, probe);
  {
    v8::TryCatch caught(isolate);
    auto script = v8::Script::Compile(context.local(),
        v8_str("Atomics.waitAsync(new Int32Array(shared),0,0)")).ToLocalChecked();
    CHECK(script->Run(context.local()).IsEmpty());
    CHECK(caught.HasTerminated());
  }
  isolate->CancelTerminateExecution();
  CHECK_EQ(1, probe->retired);
}

TEST(OrynNativeWaitRawAddressIdentityAndRetirement) {
  LocalContext context;
  auto* isolate = context.isolate();
  v8::HandleScope handles(isolate);
  auto probe = std::make_shared<WaitProbe>();
  auto backing = MakeBacking(isolate, std::make_shared<SharedLength>(),
      std::make_shared<std::atomic<int>>(0));
  auto delegate = std::make_shared<ProbeWaitDelegate>(isolate, probe);
  backing->SetSharedMemoryWaitDelegate(delegate);
  void* start = backing->Data();
  auto* address = static_cast<uint8_t*>(start) + 8;
  CHECK_EQ(1, v8::internal::FutexEmulation::Wake(address, 4));
  CHECK_EQ(8u, probe->offset);
  CHECK_EQ(1, probe->notified);
  size_t offset = 999;
  auto retained = v8::internal::BackingStore::LookupSharedWaitDelegate(address, &offset);
  CHECK_EQ(delegate.get(), retained.get());
  CHECK_EQ(8u, offset);
  CHECK(!v8::internal::BackingStore::LookupSharedWaitDelegate(
      static_cast<uint8_t*>(start) + 4096, &offset));
  // Removal must precede freeing/reusing the data mapping. Looking up the old
  // numeric address does not dereference memory and must not recover its owner.
  backing.reset();
  CHECK(!v8::internal::BackingStore::LookupSharedWaitDelegate(address, &offset));
  // In-flight notifications may hold the delegate after the registry entry dies.
  CHECK_EQ(1u, retained->Notify(8, 1));
  CHECK_EQ(2, probe->notified);
}

TEST(OrynSharedLengthAddressTracksRemoteGrowthWithoutInstanceRefresh) {
  LocalContext context;
  auto* isolate = context.isolate();
  auto length = std::make_shared<SharedLength>();
  auto backing = MakeBacking(isolate, length, std::make_shared<std::atomic<int>>(0));
  auto* internal = reinterpret_cast<v8::internal::BackingStore*>(backing.get());
  const size_t* address = internal->shared_byte_length_address();
  CHECK_EQ(length->ByteLengthAddress(), address);
  auto* atomic = reinterpret_cast<const std::atomic<size_t>*>(address);
  CHECK_EQ(16u, atomic->load(std::memory_order_acquire));
  length->length.store(128, std::memory_order_release);
  CHECK_EQ(address, internal->shared_byte_length_address());
  CHECK_EQ(128u, atomic->load(std::memory_order_acquire));
  CHECK_EQ(128u, backing->ByteLength());
}

TEST(OrynSharedWasmSizeAndExplicitBounds) {
  LocalContext context;
  auto* isolate = context.isolate();
  v8::HandleScope handles(isolate);
  // Execute this regression with Liftoff and with --no-liftoff in engine
  // qualification so both compiled size/bounds readers are exercised.
  CHECK(CompileRun(R"JS(
    (() => {
      const section=(id,body)=>[id,body.length,...body];
      const bytes=new Uint8Array([
        0,97,115,109,1,0,0,0,
        ...section(1,[2,96,0,1,127,96,1,127,1,127]),
        ...section(2,[1,3,101,110,118,3,109,101,109,2,3,1,4]),
        ...section(3,[2,0,1]),
        ...section(7,[2,4,115,105,122,101,0,0,4,108,111,97,100,0,1]),
        ...section(10,[2,4,0,63,0,11,7,0,32,0,45,0,0,11])
      ]);
      const memory=new WebAssembly.Memory({initial:1,maximum:4,shared:true});
      const instance=new WebAssembly.Instance(new WebAssembly.Module(bytes),{env:{mem:memory}});
      const {size,load}=instance.exports;
      if(size()!==1 || load(65535)!==0) return false;
      let trapped=false;try{load(65536);}catch(e){trapped=e instanceof WebAssembly.RuntimeError;}
      if(!trapped || memory.grow(1)!==1 || size()!==2 || load(65536)!==0) return false;
      new Uint8Array(memory.buffer)[65536]=93;
      if(load(65536)!==93) return false;
      for(let i=0;i<10000;i++){if(size()!==2 || load(65536)!==93)return false;}
      trapped=false;try{load(131072);}catch(e){trapped=e instanceof WebAssembly.RuntimeError;}
      return trapped;
    })()
  )JS")->IsTrue());
}

TEST(OrynSharedWasmBulkBoundsBeforeAndAfterGrowth) {
  LocalContext context;
  auto* isolate = context.isolate();
  v8::HandleScope handles(isolate);
  // Include --wasm-jitless for builds with DrumBrake, in addition to both JIT
  // tiers. Bulk operations must use the current logical size in every mode.
  CHECK(CompileRun(R"JS(
    (() => {
      const section=(id,body)=>[id,body.length,...body];
      const code=[0,32,0,32,1,32,2,252,10,0,0,11];
      const bytes=new Uint8Array([
        0,97,115,109,1,0,0,0,
        ...section(1,[1,96,3,127,127,127,0]),
        ...section(2,[1,3,101,110,118,3,109,101,109,2,3,1,4]),
        ...section(3,[1,0]),
        ...section(7,[1,4,99,111,112,121,0,0]),
        ...section(10,[1,code.length,...code])
      ]);
      const memory=new WebAssembly.Memory({initial:1,maximum:4,shared:true});
      const {copy}=new WebAssembly.Instance(new WebAssembly.Module(bytes),{env:{mem:memory}}).exports;
      let data=new Uint8Array(memory.buffer);data.fill(29,0,16);
      let trapped=false;try{copy(65530,0,10);}catch(e){trapped=e instanceof WebAssembly.RuntimeError;}
      if(!trapped || data.slice(65530).some(value=>value!==0))return false;
      copy(65536,65536,0);
      memory.grow(1);data=new Uint8Array(memory.buffer);
      copy(65530,0,10);
      if(data.slice(65530,65540).some(value=>value!==29))return false;
      trapped=false;try{copy(0,131070,4);}catch(e){trapped=e instanceof WebAssembly.RuntimeError;}
      if(!trapped || data[0]!==29 || data[1]!==29)return false;
      copy(131072,131072,0);
      return true;
    })()
  )JS")->IsTrue());
}

namespace {
class WasmLength final : public v8::SharedMemoryGrowthDelegate {
 public:
  std::atomic<size_t> length{65536};
  size_t ByteLength() const override { return length.load(std::memory_order_acquire); }
  const size_t* ByteLengthAddress() const override {
    return reinterpret_cast<const size_t*>(&length);
  }
  Result GrowAbsolute(size_t) override { return Result::kFailure; }
  bool GrowWasmByPages(size_t delta, size_t maximum, size_t* previous) override {
    size_t old = length.load(std::memory_order_acquire);
    for (;;) {
      size_t pages = old / 65536;
      if (delta > maximum || pages > maximum - delta) return false;
      if (length.compare_exchange_weak(old, (pages + delta) * 65536,
                                       std::memory_order_acq_rel)) {
        *previous = pages;
        return true;
      }
    }
  }
};
}

TEST(OrynNativeSharedWasmDeltaGrowthAndMappingOwnership) {
  LocalContext context;
  auto* isolate = context.isolate();
  auto releases = std::make_shared<std::atomic<int>>(0);
  auto length = std::make_shared<WasmLength>();
  {
    auto storage = v8::SharedArrayBuffer::NewBackingStore(isolate, 4 * 65536);
    void* data = storage->Data();
    auto backing = v8::BackingStore::NewSharedWasmBackingStore(data, 65536,
        4 * 65536, 4 * 65536, false, length, Release,
        new Allocation{std::move(storage), releases});
    CHECK_NOT_NULL(backing.get());
    auto* internal = reinterpret_cast<v8::internal::BackingStore*>(backing.get());
    CHECK(internal->is_wasm_memory());
    CHECK(internal->is_shared());
    CHECK(!internal->has_guard_regions());
    std::optional<size_t> first, second;
    std::thread a([&] { first = internal->GrowWasmMemoryInPlace(nullptr, 1, 4); });
    std::thread b([&] { second = internal->GrowWasmMemoryInPlace(nullptr, 1, 4); });
    a.join(); b.join();
    CHECK(first.has_value()); CHECK(second.has_value());
    CHECK((*first == 1 && *second == 2) || (*first == 2 && *second == 1));
    CHECK_EQ(3u * 65536, backing->ByteLength());
    CHECK_EQ(3u, *internal->GrowWasmMemoryInPlace(nullptr, 0, 4));
    CHECK(!internal->GrowWasmMemoryInPlace(nullptr, 2, 4).has_value());
    CHECK_EQ(3u * 65536, backing->ByteLength());
    CHECK_EQ(0, releases->load());
  }
  CHECK_EQ(1, releases->load());
  {
    auto storage = v8::SharedArrayBuffer::NewBackingStore(isolate, 65536);
    void* data = storage->Data();
    auto invalid = v8::BackingStore::NewSharedWasmBackingStore(data, 1,
        65536, 65536, false, length, Release,
        new Allocation{std::move(storage), releases});
    CHECK_NULL(invalid.get());
  }
  CHECK_EQ(2, releases->load());
}

namespace {
struct FactoryProbe final : v8::SharedMemoryAllocationDelegate {
  unsigned calls = 0;
  bool reject = false;
  bool wrong_kind = false;
  bool saw_wasm = false;
  size_t initial = 0;
  size_t maximum = 0;
  std::shared_ptr<v8::BackingStore> AllocateSharedMemory(
      v8::Isolate* isolate, size_t bytes, size_t max, bool growable,
      bool wasm, bool memory64) override {
    ++calls;
    initial = bytes;
    maximum = max;
    saw_wasm = wasm;
    if (reject || wasm) return {};
    if (wrong_kind) return v8::ArrayBuffer::NewBackingStore(isolate, bytes);
    if (!growable) return v8::SharedArrayBuffer::NewBackingStore(isolate, bytes);
    auto releases = std::make_shared<std::atomic<int>>(0);
    auto length = std::make_shared<SharedLength>();
    length->length.store(bytes);
    return MakeBacking(isolate, length, releases, max);
  }
};
struct FactoryScope {
  v8::Isolate* isolate;
  explicit FactoryScope(v8::Isolate* value,
                        std::shared_ptr<FactoryProbe> probe) : isolate(value) {
    isolate->SetSharedMemoryAllocationDelegate(std::move(probe));
  }
  ~FactoryScope() { isolate->SetSharedMemoryAllocationDelegate({}); }
};
}

TEST(OrynSharedFactoryConstructorAuthority) {
  LocalContext context;
  auto* isolate = context.isolate();
  v8::HandleScope handles(isolate);
  auto probe = std::make_shared<FactoryProbe>();
  FactoryScope factory(isolate, probe);
  CHECK(CompileRun("new ArrayBuffer(16).byteLength === 16")->IsTrue());
  CHECK_EQ(0u, probe->calls);
  CHECK(CompileRun("new SharedArrayBuffer(16).byteLength === 16")->IsTrue());
  CHECK_EQ(1u, probe->calls);
  CHECK(CompileRun("globalThis.factoryBuffer = new SharedArrayBuffer(16,"
                   " {maxByteLength:4096});"
                   "factoryBuffer.grow(64);"
                   "factoryBuffer.byteLength === 64 && factoryBuffer.growable")
            ->IsTrue());
  CHECK_EQ(2u, probe->calls);
  CHECK_EQ(4096u, probe->maximum);
  CHECK(CompileRun("try { new SharedArrayBuffer(16,{maxByteLength:8}); false }"
                   "catch(e) { e instanceof RangeError }")->IsTrue());
  CHECK_EQ(2u, probe->calls);
  probe->reject = true;
  // The normal prototype lookup still occurs after allocation failure.
  CHECK(CompileRun("globalThis.prototypeReads = 0;"
                   "const target = new Proxy(function(){}, {get(t,k,r){"
                   "if(k==='prototype') ++prototypeReads; return Reflect.get(t,k,r)}});"
                   "try { Reflect.construct(SharedArrayBuffer,[16],target); false }"
                   "catch(e) { e instanceof RangeError && prototypeReads === 1 }")
            ->IsTrue());
  CHECK_EQ(3u, probe->calls);
  probe->reject = false;
  probe->wrong_kind = true;
  CHECK(CompileRun("try { new SharedArrayBuffer(16); false }"
                   "catch(e) { e instanceof RangeError }")->IsTrue());
  CHECK_EQ(4u, probe->calls);
}

TEST(OrynSharedFactoryWasmFailureDoesNotFallback) {
  LocalContext context;
  auto* isolate = context.isolate();
  v8::HandleScope handles(isolate);
  auto probe = std::make_shared<FactoryProbe>();
  FactoryScope factory(isolate, probe);
  CHECK(CompileRun("new WebAssembly.Memory({initial:1,maximum:2}).buffer.byteLength"
                   " === 65536")->IsTrue());
  CHECK_EQ(0u, probe->calls);
  CHECK(CompileRun("try { new WebAssembly.Memory({initial:1,maximum:2,shared:true});"
                   "false } catch(e) { e instanceof RangeError }")->IsTrue());
  CHECK_EQ(1u, probe->calls);
  CHECK(probe->saw_wasm);
  CHECK_EQ(65536u, probe->initial);
  CHECK_EQ(131072u, probe->maximum);
}

namespace {
struct NativeWasmFactory final : v8::SharedMemoryAllocationDelegate {
  std::shared_ptr<WasmLength> length = std::make_shared<WasmLength>();
  std::shared_ptr<std::atomic<int>> releases = std::make_shared<std::atomic<int>>(0);
  unsigned allocations = 0;
  std::shared_ptr<v8::BackingStore> AllocateSharedMemory(
      v8::Isolate* isolate, size_t initial, size_t maximum,
      bool growable, bool wasm, bool memory64) override {
    if (!wasm || memory64 || initial != 65536 || maximum != 4 * 65536)
      return {};
    ++allocations;
    auto storage = v8::SharedArrayBuffer::NewBackingStore(isolate, maximum);
    if (!storage) return {};
    void* data = storage->Data();
    return v8::BackingStore::NewSharedWasmBackingStore(data, initial, maximum,
        maximum, false, length, Release,
        new Allocation{std::move(storage), releases});
  }
};
struct WasmCloneDelegate final : v8::ValueSerializer::Delegate,
                               v8::ValueDeserializer::Delegate {
  v8::Isolate* isolate;
  std::shared_ptr<v8::BackingStore> backing;
  bool substitute_ordinary = false;
  explicit WasmCloneDelegate(v8::Isolate* value) : isolate(value) {}
  void ThrowDataCloneError(v8::Local<v8::String> message) override {
    isolate->ThrowException(v8::Exception::Error(message));
  }
  v8::Maybe<uint32_t> GetSharedArrayBufferId(
      v8::Isolate*, v8::Local<v8::SharedArrayBuffer> buffer) override {
    backing = buffer->GetBackingStore();
    return v8::Just(uint32_t{0});
  }
  v8::MaybeLocal<v8::SharedArrayBuffer> GetSharedArrayBufferFromId(
      v8::Isolate* isolate, uint32_t id) override {
    if (id != 0 || !backing) return {};
    if (substitute_ordinary) {
      return v8::SharedArrayBuffer::New(isolate, 65536);
    }
    return v8::SharedArrayBuffer::New(isolate, backing);
  }
};
}

TEST(OrynNativeSharedWasmConstructCloneImportAndRemoteGrowth) {
  LocalContext context;
  auto* isolate = context.isolate();
  v8::HandleScope handles(isolate);
  auto factory = std::make_shared<NativeWasmFactory>();
  isolate->SetSharedMemoryAllocationDelegate(factory);
  auto memory = CompileRun(
      "globalThis.nativeMemory = new WebAssembly.Memory({initial:1,maximum:4,shared:true});"
      "new Uint8Array(nativeMemory.buffer)[3]=71; nativeMemory");
  CHECK_EQ(1u, factory->allocations);
  WasmCloneDelegate delegate(isolate);
  v8::ValueSerializer serializer(isolate, &delegate);
  serializer.WriteHeader();
  CHECK(serializer.WriteValue(context.local(), memory).FromJust());
  auto bytes = serializer.Release();
  v8::ValueDeserializer deserializer(isolate, bytes.first, bytes.second, &delegate);
  CHECK(deserializer.ReadHeader(context.local()).FromJust());
  auto clone = deserializer.ReadValue(context.local()).ToLocalChecked();
  CHECK(context->Global()->Set(context.local(), v8_str("clonedMemory"), clone).FromJust());
  CHECK_EQ(1u, factory->allocations);  // Deserialization reuses the admitted backing.
  CHECK(CompileRun(R"JS(
    (() => {
      if (!(clonedMemory instanceof WebAssembly.Memory) || clonedMemory === nativeMemory)
        return false;
      if(new Uint8Array(clonedMemory.buffer)[3]!==71)return false;
      const section=(id,body)=>[id,body.length,...body];
      const bytes=new Uint8Array([
        0,97,115,109,1,0,0,0,
        ...section(1,[2,96,0,1,127,96,1,127,1,127]),
        ...section(2,[1,3,101,110,118,3,109,101,109,2,3,1,4]),
        ...section(3,[2,0,1]),
        ...section(7,[2,4,115,105,122,101,0,0,4,108,111,97,100,0,1]),
        ...section(10,[2,4,0,63,0,11,7,0,32,0,45,0,0,11])
      ]);
      globalThis.nativeExports=new WebAssembly.Instance(new WebAssembly.Module(bytes),
        {env:{mem:clonedMemory}}).exports;
      if(nativeExports.size()!==1 || nativeExports.load(3)!==71)return false;
      if(clonedMemory.grow(1)!==1 || nativeExports.size()!==2)return false;
      return nativeMemory.buffer.byteLength===131072;
    })()
  )JS")->IsTrue());
  // Another native process publishes this same metadata cell without refreshing
  // this isolate's instance caches. Every compiled bounds/size read must see it.
  CHECK(CompileRun("globalThis.beforeRemote = clonedMemory.buffer; beforeRemote.byteLength===131072")->IsTrue());
  factory->length->length.store(3 * 65536, std::memory_order_release);
  CHECK(CompileRun("nativeExports.size()===3 && nativeExports.load(131072)===0 &&"
                   "clonedMemory.buffer.byteLength===196608 &&"
                   "clonedMemory.buffer!==beforeRemote && beforeRemote.byteLength===131072")
            ->IsTrue());
  CHECK(CompileRun("try { nativeExports.load(196608); false }"
                   "catch(e) { e instanceof WebAssembly.RuntimeError }")->IsTrue());
  // A delegate/wire kind mismatch must reject before the Wasm union is accessed.
  delegate.substitute_ordinary = true;
  {
    v8::TryCatch caught(isolate);
    v8::ValueDeserializer malformed(isolate, bytes.first, bytes.second, &delegate);
    CHECK(malformed.ReadHeader(context.local()).FromJust());
    CHECK(malformed.ReadValue(context.local()).IsEmpty());
  }
  free(bytes.first);
  isolate->SetSharedMemoryAllocationDelegate({});
}
