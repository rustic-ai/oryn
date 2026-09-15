// Private, isolate-owned algorithm context. No JS callbacks, DOM pointers,
// network services or C++ objects are exchanged across the C ABI.
#include "v8-context.h"
#include "v8-exception.h"
#include "v8-isolate.h"
#include "v8-persistent-handle.h"
#include "v8-primitive.h"
#include "v8-regexp.h"
#include <deque>
#include <memory>
#include <new>
#include <string>

namespace {
constexpr size_t kPatternLimit = 16 * 1024;
constexpr size_t kValueLimit = 1024 * 1024;
constexpr size_t kCacheUnits = 128 * 1024;
constexpr size_t kCacheEntries = 64;
constexpr uint32_t kBacktrackLimit = 100000;
struct Entry {
  std::u16string pattern;
  v8::Global<v8::RegExp> regexp;  // Empty means invalid source syntax.
};
struct Service {
  explicit Service(v8::Isolate* owner) : isolate(owner) {}
  v8::Isolate* isolate;
  v8::Global<v8::Context> context;
  std::deque<Entry> cache;
  size_t cached_units = 0;
};
}

extern "C" void* oryn_v8_pattern_service_new(v8::Isolate* isolate) {
  return new (std::nothrow) Service(isolate);
}
extern "C" void oryn_v8_pattern_service_delete(void* pointer) {
  auto* service = static_cast<Service*>(pointer);
  v8::Isolate::Scope entered(service->isolate);
  delete service;
}

// 0=no match (including the V8 backtrack ceiling), 1=match, 2=invalid
// pattern syntax, -1=engine/resource failure. Never turn failure into a pass.
extern "C" int32_t oryn_v8_pattern_match(void* pointer,
    const uint16_t* pattern, size_t pattern_len,
    const uint16_t* value, size_t value_len) {
  if (pattern_len > kPatternLimit || value_len > kValueLimit) return -1;
  auto* service = static_cast<Service*>(pointer);
  auto* isolate = service->isolate;
  v8::Isolate::Scope entered(isolate);
  v8::HandleScope handles(isolate);
  v8::TryCatch errors(isolate);
  if (isolate->IsExecutionTerminating()) return -1;
  if (service->context.IsEmpty()) {
    auto context = v8::Context::New(isolate);
    if (context.IsEmpty()) return -1;
    context->AllowCodeGenerationFromStrings(false);
    service->context.Reset(isolate, context);
  }
  auto context = service->context.Get(isolate);
  v8::Context::Scope entered_context(context);
  // Use an inaccessible context because RegExp::Exec updates RegExp statics.
  // The context and all compiled expressions die with the agent isolate.
  std::u16string key;
  key.reserve(pattern_len);
  for (size_t i = 0; i < pattern_len; ++i) key.push_back(pattern[i]);
  Entry* entry = nullptr;
  for (auto& cached : service->cache) {
    if (cached.pattern == key) { entry = &cached; break; }
  }
  if (!entry) {
    while (!service->cache.empty() &&
        (service->cache.size() >= kCacheEntries || service->cached_units + pattern_len > kCacheUnits)) {
      service->cached_units -= service->cache.front().pattern.size();
      service->cache.pop_front();
    }
    v8::Local<v8::String> source;
    if (!v8::String::NewFromTwoByte(isolate, pattern, v8::NewStringType::kNormal,
        static_cast<int>(pattern_len)).ToLocal(&source)) return -1;
    v8::Local<v8::RegExp> regexp;
    // Validate the original source before wrapping, so e.g. "a)(b" cannot
    // escape the noncapturing group and become a valid anchored expression.
    if (!v8::RegExp::NewWithBacktrackLimit(context, source,
        v8::RegExp::kUnicodeSets, kBacktrackLimit).ToLocal(&regexp)) {
      if (isolate->IsExecutionTerminating() || !errors.HasCaught()) return -1;
      errors.Reset();
      service->cache.push_back(Entry{std::move(key), {}});
      service->cached_units += pattern_len;
      return 2;
    }
    auto anchored = std::u16string(u"^(?:") + key + u")$";
    if (!v8::String::NewFromTwoByte(isolate,
        reinterpret_cast<const uint16_t*>(anchored.data()), v8::NewStringType::kNormal,
        static_cast<int>(anchored.size())).ToLocal(&source)) return -1;
    if (!v8::RegExp::NewWithBacktrackLimit(context, source,
        v8::RegExp::kUnicodeSets, kBacktrackLimit).ToLocal(&regexp)) return -1;
    service->cache.push_back(Entry{std::move(key), v8::Global<v8::RegExp>(isolate, regexp)});
    service->cached_units += pattern_len;
    entry = &service->cache.back();
  }
  if (entry->regexp.IsEmpty()) return 2;
  v8::Local<v8::String> subject;
  if (!v8::String::NewFromTwoByte(isolate, value, v8::NewStringType::kNormal,
      static_cast<int>(value_len)).ToLocal(&subject)) return -1;
  v8::Local<v8::Object> result;
  if (!entry->regexp.Get(isolate)->Exec(context, subject).ToLocal(&result)) return -1;
  return result->IsNull() ? 0 : 1;
}
