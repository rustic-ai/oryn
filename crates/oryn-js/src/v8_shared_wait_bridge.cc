// Native Atomics wait scheduling. Pending promises belong to one environment;
// neither a page global nor an authored Atomics replacement is involved.
#include "v8.h"
#include <algorithm>
#include <atomic>
#include <memory>
#include <vector>

extern "C" void* oryn_v8_current_wait_environment(v8::Local<v8::Context>);
namespace {
using BeginCallback = void* (*)(void*, v8::Local<v8::Context>, size_t, bool, uint64_t, int64_t, int*);
using PollCallback = int (*)(void*, uint32_t);
using CancelCallback = void (*)(void*);
using NotifyCallback = uint32_t (*)(void*, size_t, uint32_t);
using ReleaseCallback = void (*)(void*);
using State = v8::SharedMemoryWaiter::State;
struct NativeWait final : v8::SharedMemoryWaiter {
  void* context; PollCallback poll; PollCallback admission; CancelCallback cancel;
  State initial_state;
  NativeWait(void* c, PollCallback p, PollCallback a, CancelCallback d, State initial)
      : context(c), poll(p), admission(a), cancel(d), initial_state(initial) {}
  ~NativeWait() override { cancel(context); }
  State Poll(uint32_t milliseconds) override {
    return Decode(poll(context, std::min(milliseconds, uint32_t{50})));
  }
  State Admission(uint32_t milliseconds) {
    if (initial_state == State::kAdmitting) {
      initial_state = Decode(admission(context, std::min(milliseconds, uint32_t{50})));
    }
    return initial_state;
  }
  static State Decode(int result) {
    switch (result) {
      case 0: return State::kPending;
      case 1: return State::kOk;
      case 2: return State::kNotEqual;
      case 3: return State::kTimedOut;
      case 5: return State::kAdmitting;
      default: return State::kCancelled;
    }
  }
};
struct Pending {
  std::unique_ptr<v8::SharedMemoryWaiter> waiter;
  v8::Global<v8::Context> context;
  v8::Global<v8::Promise::Resolver> resolver;
};
struct Environment {
  std::atomic<bool> retired{false};
  std::vector<Pending> pending;
  void Retire() { retired = true; pending.clear(); }
};
using EnvironmentHandle = std::shared_ptr<Environment>;
const char* Result(State state) {
  return state == State::kOk ? "ok" : state == State::kNotEqual ? "not-equal" : "timed-out";
}
struct Delegate final : v8::SharedMemoryWaitDelegate {
  EnvironmentHandle environment;
  void* context; BeginCallback begin; PollCallback poll; PollCallback admission; CancelCallback cancel; NotifyCallback notify; ReleaseCallback release;
  Delegate(EnvironmentHandle e, void* c, BeginCallback b, PollCallback p, PollCallback a, CancelCallback d, NotifyCallback n, ReleaseCallback r)
      : environment(std::move(e)), context(c), begin(b), poll(p), admission(a), cancel(d), notify(n), release(r) {}
  ~Delegate() override { release(context); }
  std::unique_ptr<v8::SharedMemoryWaiter> Begin(size_t offset, bool width64,
      uint64_t expected, int64_t timeout) override {
    int initial = 4;
    auto isolate = v8::Isolate::GetCurrent();
    if (!isolate || isolate->GetCurrentContext().IsEmpty()) return {};
    void* waiter = begin(context, isolate->GetCurrentContext(), offset, width64, expected, timeout, &initial);
    if (!waiter) return {};
    return std::make_unique<NativeWait>(waiter, poll, admission, cancel, NativeWait::Decode(initial));
  }
  v8::MaybeLocal<v8::Object> WaitAsync(v8::Isolate* isolate, size_t offset,
      bool width64, uint64_t expected, int64_t timeout,
      ProcessInterrupts process_interrupts, void* interrupt_context) override {
    auto entered = isolate->GetCurrentContext();
    void* handle = oryn_v8_current_wait_environment(entered);
    if (!handle) return {};
    auto environment = *static_cast<EnvironmentHandle*>(handle);
    if (environment->retired) return {};
    auto waiter = Begin(offset, width64, expected, timeout);
    if (!waiter) return {};
    // Classification belongs to the atomic compare/enqueue operation. A remote
    // notify before this object is constructed still settles an async promise.
    auto state = State::kAdmitting;
    while (state == State::kAdmitting) {
      if (!process_interrupts(interrupt_context)) return {};
      if (environment->retired) return {};
      state = static_cast<NativeWait*>(waiter.get())->Admission(50);
      if (environment->retired) return {};
    }
    if (state == State::kCancelled || state == State::kOk) return {};
    auto context = isolate->GetCurrentContext();
    auto result = v8::Object::New(isolate);
    bool asynchronous = state == State::kPending;
    auto key_async = v8::String::NewFromUtf8Literal(isolate, "async");
    auto key_value = v8::String::NewFromUtf8Literal(isolate, "value");
    if (!result->CreateDataProperty(context, key_async, v8::Boolean::New(isolate, asynchronous)).FromMaybe(false)) return {};
    v8::Local<v8::Value> value;
    v8::Local<v8::Promise::Resolver> resolver;
    if (asynchronous) {
      if (environment->pending.size() >= 4096 ||
          !v8::Promise::Resolver::New(context).ToLocal(&resolver)) return {};
      value = resolver->GetPromise();
    } else {
      value = v8::String::NewFromUtf8(isolate, Result(state)).ToLocalChecked();
    }
    if (!result->CreateDataProperty(context, key_value, value).FromMaybe(false)) return {};
    if (asynchronous) environment->pending.push_back(Pending{
        std::move(waiter), v8::Global<v8::Context>(isolate, context),
        v8::Global<v8::Promise::Resolver>(isolate, resolver)});
    return result;
  }
  uint32_t Notify(size_t offset, uint32_t count) override {
    return notify(context, offset, count);
  }
};
}
extern "C" void* oryn_v8_wait_environment_new() {
  return new EnvironmentHandle(std::make_shared<Environment>());
}
extern "C" void oryn_v8_wait_environment_retire(void* pointer) {
  if (pointer) (*static_cast<EnvironmentHandle*>(pointer))->Retire();
}
extern "C" void oryn_v8_wait_environment_delete(void* pointer) {
  if (!pointer) return;
  oryn_v8_wait_environment_retire(pointer);
  delete static_cast<EnvironmentHandle*>(pointer);
}
extern "C" bool oryn_v8_wait_environment_pending(void* pointer) {
  auto environment = *static_cast<EnvironmentHandle*>(pointer);
  return !environment->retired && !environment->pending.empty();
}
extern "C" size_t oryn_v8_wait_environment_poll(v8::Isolate* isolate, void* pointer) {
  auto environment = *static_cast<EnvironmentHandle*>(pointer);
  if (environment->retired) return 0;
  size_t completed = 0;
  // Extract terminal records before settling promises; native callbacks cannot
  // invalidate an iterator or resurrect a waiter already acknowledged.
  std::vector<Pending> ready;
  std::vector<State> states;
  for (auto iterator = environment->pending.begin(); iterator != environment->pending.end();) {
    auto state = iterator->waiter->Poll(0);
    if (state == State::kPending || state == State::kAdmitting) { ++iterator; continue; }
    states.push_back(state); ready.push_back(std::move(*iterator));
    iterator = environment->pending.erase(iterator);
  }
  for (size_t i = 0; i < ready.size(); ++i) {
    if (environment->retired || states[i] == State::kCancelled) continue;
    v8::HandleScope handles(isolate);
    auto context = ready[i].context.Get(isolate);
    v8::Context::Scope entered(context);
    auto value = v8::String::NewFromUtf8(isolate, Result(states[i])).ToLocalChecked();
    if (ready[i].resolver.Get(isolate)->Resolve(context, value).FromMaybe(false)) ++completed;
  }
  return completed;
}
extern "C" void oryn_v8_set_shared_wait_delegate(const v8::SharedArrayBuffer* buffer,
    void* environment, void* context, BeginCallback begin, PollCallback poll, PollCallback admission, CancelCallback cancel,
    NotifyCallback notify, ReleaseCallback release) {
  auto delegate = std::make_shared<Delegate>(*static_cast<EnvironmentHandle*>(environment),
      context, begin, poll, admission, cancel, notify, release);
  const_cast<v8::SharedArrayBuffer*>(buffer)->GetBackingStore()
      ->SetSharedMemoryWaitDelegate(std::move(delegate));
}
