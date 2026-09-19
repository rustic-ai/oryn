// Source-only regression for the frame-context-reattachment patch.
#include "test/cctest/cctest.h"
#include "include/v8-context.h"
#include "include/v8-function.h"
#include "include/v8-microtask-queue.h"
TEST(OrynPreservedFrameContextReattachesOriginalWindowProxy) {
  v8::Isolate* isolate = CcTest::isolate();
  v8::HandleScope handles(isolate);
  auto queue = v8::MicrotaskQueue::New(isolate, v8::MicrotasksPolicy::kExplicit);
  auto original = v8::Context::New(isolate, nullptr, {}, {}, {}, queue.get());
  auto proxy = original->Global();
  {
    v8::Context::Scope entered(original);
    CompileRun("let keptLexical = 17; globalThis.keptObject = { value: 29 }");
  }
  CHECK(!original->ReattachGlobal(queue.get()).FromJust());
  original->DetachGlobal();
  auto replacement = v8::Context::New(isolate, nullptr, {}, proxy, {}, queue.get());
  CHECK(replacement->Global()->StrictEquals(proxy));
  CHECK(!original->ReattachGlobal(queue.get()).FromJust());
  replacement->DetachGlobal();
  CHECK(original->ReattachGlobal(queue.get()).FromJust());
  CHECK(original->Global()->StrictEquals(proxy));
  {
    v8::Context::Scope entered(original);
    CHECK(CompileRun("keptLexical === 17 && keptObject.value === 29")->IsTrue());
  }
}

TEST(OrynCachedRootRetainsPromiseJobsUntilSharedQueueRestoration) {
  v8::Isolate* isolate = CcTest::isolate();
  v8::HandleScope handles(isolate);
  const auto previous_policy = isolate->GetMicrotasksPolicy();
  isolate->SetMicrotasksPolicy(v8::MicrotasksPolicy::kExplicit);
  auto original = v8::Context::New(isolate);
  auto paused = v8::MicrotaskQueue::New(isolate, v8::MicrotasksPolicy::kExplicit);
  v8::Local<v8::Function> finish;
  {
    v8::Context::Scope entered(original);
    finish = CompileRun("let deliveries = []; let complete; new Promise(r => complete = r).then(() => deliveries.push(1)); complete").As<v8::Function>();
  }
  isolate->PerformMicrotaskCheckpoint();
  CHECK(original->SuspendMicrotasks(paused.get()).FromJust());
  auto proxy = original->Global();
  original->DetachGlobalPreservingMicrotasks();
  auto replacement = v8::Context::New(isolate, nullptr, {}, proxy);
  {
    v8::Context::Scope entered(replacement);
    CHECK(!finish->Call(replacement, v8::Undefined(isolate), 0, nullptr).IsEmpty());
  }
  isolate->PerformMicrotaskCheckpoint();
  {
    v8::Context::Scope entered(original);
    CHECK_EQ(0, CompileRun("deliveries.length")->Int32Value(original).FromJust());
  }
  replacement->DetachGlobal();
  CHECK(original->ReattachGlobal().FromJust());
  CHECK(original->ResumeMicrotasks(paused.get(), false).FromJust());
  isolate->PerformMicrotaskCheckpoint();
  {
    v8::Context::Scope entered(original);
    CHECK_EQ(1, CompileRun("deliveries.length")->Int32Value(original).FromJust());
  }
  CHECK(!original->ResumeMicrotasks(paused.get(), false).FromJust());
  original->DetachGlobal();
  isolate->SetMicrotasksPolicy(previous_policy);
}

TEST(OrynSuspensionRejectsEnteredContextAndPendingSharedJobs) {
  v8::Isolate* isolate = CcTest::isolate();
  v8::HandleScope handles(isolate);
  const auto previous_policy = isolate->GetMicrotasksPolicy();
  isolate->SetMicrotasksPolicy(v8::MicrotasksPolicy::kExplicit);
  auto context = v8::Context::New(isolate);
  auto paused = v8::MicrotaskQueue::New(isolate, v8::MicrotasksPolicy::kExplicit);
  {
    v8::Context::Scope entered(context);
    CHECK(!context->SuspendMicrotasks(paused.get()).FromJust());
    CompileRun("Promise.resolve().then(() => 1)");
  }
  CHECK(!context->SuspendMicrotasks(paused.get()).FromJust());
  isolate->PerformMicrotaskCheckpoint();
  CHECK(context->SuspendMicrotasks(paused.get()).FromJust());
  CHECK(context->ResumeMicrotasks(paused.get(), true).FromJust());
  context->DetachGlobal();
  isolate->SetMicrotasksPolicy(previous_policy);
}
