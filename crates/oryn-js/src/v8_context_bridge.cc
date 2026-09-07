// These wrappers expose public V8 APIs through the same opaque local-handle
// convention used by rusty_v8's binding.cc. No C++ objects cross the C ABI.
#include "v8-context.h"
#include "v8-isolate.h"
#include "v8-version.h"

static_assert(sizeof(v8::Local<v8::Context>) == sizeof(void*));

extern "C" const v8::Context* oryn_v8_incumbent_context(v8::Isolate* isolate) {
  auto context = isolate->GetIncumbentContext();
  return context.IsEmpty() ? nullptr : *context;
}

extern "C" void oryn_v8_detach_global(v8::Context* context) {
  context->DetachGlobal();
}

// Security tokens are embedder-owned and include distinct identities for opaque
// origins. Read the caller's incumbent context, not the native function's realm.
extern "C" bool oryn_v8_can_access_window(v8::Isolate* isolate,
                                         v8::Object* receiver) {
  auto caller = isolate->GetIncumbentContext();
  v8::Local<v8::Context> target;
  if (caller.IsEmpty() || !receiver->GetCreationContext().ToLocal(&target) ||
      !receiver->StrictEquals(target->Global())) {
    return false;
  }
  return caller->GetSecurityToken()->StrictEquals(target->GetSecurityToken());
}
