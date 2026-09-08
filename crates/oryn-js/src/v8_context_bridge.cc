// These wrappers expose public V8 APIs through the same opaque local-handle
// convention used by rusty_v8's binding.cc. No C++ objects cross the C ABI.
#include "v8-context.h"
#include "v8-isolate.h"
#include "v8-version.h"
#include "v8-template.h"

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

struct OrynCrossContextHandlers {
  v8::NamedPropertyGetterCallback named_getter;
  v8::NamedPropertySetterCallback named_setter;
  v8::NamedPropertyQueryCallback named_query;
  v8::NamedPropertyDeleterCallback named_deleter;
  v8::NamedPropertyEnumeratorCallback named_enumerator;
  v8::NamedPropertyDefinerCallback named_definer;
  v8::NamedPropertyDescriptorCallback named_descriptor;
  v8::IndexedPropertyGetterCallbackV2 indexed_getter;
  v8::IndexedPropertySetterCallbackV2 indexed_setter;
  v8::IndexedPropertyQueryCallbackV2 indexed_query;
  v8::IndexedPropertyDeleterCallbackV2 indexed_deleter;
  v8::IndexedPropertyEnumeratorCallback indexed_enumerator;
  v8::IndexedPropertyDefinerCallbackV2 indexed_definer;
  v8::IndexedPropertyDescriptorCallbackV2 indexed_descriptor;
};

static bool same_security_token(v8::Local<v8::Context> caller,
                                v8::Local<v8::Object> receiver,
                                v8::Local<v8::Value>) {
  v8::Local<v8::Context> target;
  return receiver->GetCreationContext().ToLocal(&target) &&
         caller->GetSecurityToken()->StrictEquals(target->GetSecurityToken());
}

extern "C" void oryn_v8_set_cross_context_handlers(
    v8::ObjectTemplate* object_template, const OrynCrossContextHandlers* h) {
  object_template->SetAccessCheckCallbackAndHandler(
      same_security_token,
      v8::NamedPropertyHandlerConfiguration(
          h->named_getter, h->named_setter, h->named_query, h->named_deleter,
          h->named_enumerator, h->named_definer, h->named_descriptor),
      v8::IndexedPropertyHandlerConfiguration(
          h->indexed_getter, h->indexed_setter, h->indexed_query,
          h->indexed_deleter, h->indexed_enumerator, h->indexed_definer,
          h->indexed_descriptor));
}

extern "C" void oryn_v8_window_access_denied(v8::Local<v8::Context>, v8::Local<v8::Object>);
static void window_access_denied(v8::Local<v8::Object> target, v8::AccessType,
                                 v8::Local<v8::Value>) {
  oryn_v8_window_access_denied(v8::Isolate::GetCurrent()->GetCurrentContext(), target);
}
extern "C" void oryn_v8_set_window_failed_access_callback(v8::Isolate* isolate) {
  isolate->SetFailedAccessCheckCallbackFunction(window_access_denied);
}

// WindowProxy rejects every indexed assignment/definition. Use the modern
// boolean callbacks: rusty_v8 currently exposes the legacy void signatures,
// which cannot report a false completion without throwing an exception.
static v8::Intercepted window_index_set(
    uint32_t, v8::Local<v8::Value>, const v8::PropertyCallbackInfo<v8::Boolean>& info) {
  info.GetReturnValue().Set(false);
  return v8::Intercepted::kYes;
}
static v8::Intercepted window_index_define(
    uint32_t, const v8::PropertyDescriptor&,
    const v8::PropertyCallbackInfo<v8::Boolean>& info) {
  info.GetReturnValue().Set(false);
  return v8::Intercepted::kYes;
}
extern "C" void oryn_v8_set_window_index_handlers(
    v8::ObjectTemplate* object_template, const OrynCrossContextHandlers* h) {
  object_template->SetWindowProxyStructuralSemantics();
  object_template->SetHandler(v8::IndexedPropertyHandlerConfiguration(
      h->indexed_getter, window_index_set, h->indexed_query,
      h->indexed_deleter, h->indexed_enumerator, window_index_define,
      h->indexed_descriptor));
}

// HTMLAllCollection is a callable legacy object with [[IsHTMLDDA]]. These
// public V8 template APIs provide its language-level behavior directly.
extern "C" void oryn_v8_configure_html_all_collection(
    v8::ObjectTemplate* object_template, v8::FunctionCallback callback) {
  object_template->MarkAsUndetectable();
  object_template->SetCallAsFunctionHandler(callback);
}
