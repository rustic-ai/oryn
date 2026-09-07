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
