// HTML cross-origin own-key enumeration, with unrelated access checks retained.
#include "test/cctest/cctest.h"
#include "include/v8-template.h"
#include "include/v8-container.h"

namespace {
bool OrynEnumerationSameContext(v8::Local<v8::Context> caller,
                                v8::Local<v8::Object> receiver,
                                v8::Local<v8::Value>) {
  v8::Local<v8::Context> owner;
  return receiver->GetCreationContext().ToLocal(&owner) && owner == caller;
}
v8::Intercepted OrynEnumerationNamedGet(v8::Local<v8::Name> name,
    const v8::PropertyCallbackInfo<v8::Value>& info) {
  if (!name->StrictEquals(v8_str("hidden"))) return v8::Intercepted::kNo;
  info.GetReturnValue().Set(17);
  return v8::Intercepted::kYes;
}
v8::Intercepted OrynEnumerationNamedQuery(v8::Local<v8::Name> name,
    const v8::PropertyCallbackInfo<v8::Integer>& info) {
  if (!name->StrictEquals(v8_str("hidden"))) return v8::Intercepted::kNo;
  info.GetReturnValue().Set(v8::DontEnum);
  return v8::Intercepted::kYes;
}
void OrynEnumerationNames(const v8::PropertyCallbackInfo<v8::Array>& info) {
  v8::Local<v8::Value> keys[] = {v8_str("hidden")};
  info.GetReturnValue().Set(v8::Array::New(info.GetIsolate(), keys, 1));
}
v8::Intercepted OrynEnumerationIndexedGet(uint32_t index,
    const v8::PropertyCallbackInfo<v8::Value>& info) {
  if (index != 0) return v8::Intercepted::kNo;
  info.GetReturnValue().Set(29);
  return v8::Intercepted::kYes;
}
v8::Intercepted OrynEnumerationIndexedQuery(uint32_t index,
    const v8::PropertyCallbackInfo<v8::Integer>& info) {
  if (index != 0) return v8::Intercepted::kNo;
  info.GetReturnValue().Set(v8::ReadOnly);
  return v8::Intercepted::kYes;
}
void OrynEnumerationIndices(const v8::PropertyCallbackInfo<v8::Array>& info) {
  v8::Local<v8::Value> keys[] = {v8::Integer::New(info.GetIsolate(), 0)};
  info.GetReturnValue().Set(v8::Array::New(info.GetIsolate(), keys, 1));
}
void CheckOrynCrossOriginEnumeration(bool opted_in) {
  auto* isolate = CcTest::isolate();
  v8::HandleScope handles(isolate);
  auto owner = v8::Context::New(isolate);
  v8::Local<v8::Object> target;
  {
    v8::Context::Scope entered(owner);
    // Install the inherited sentinel as the initial prototype, before the
    // instance opts into JavaScript-visible immutable prototype semantics.
    auto constructor_template = v8::FunctionTemplate::New(isolate);
    constructor_template->PrototypeTemplate()->Set(
        v8_str("must_not_leak"), v8::True(isolate));
    auto object_template = constructor_template->InstanceTemplate();
    if (opted_in) object_template->SetWindowProxyStructuralSemantics();
    object_template->SetAccessCheckCallbackAndHandler(
        OrynEnumerationSameContext,
        v8::NamedPropertyHandlerConfiguration(OrynEnumerationNamedGet, nullptr,
            OrynEnumerationNamedQuery, nullptr, OrynEnumerationNames),
        v8::IndexedPropertyHandlerConfiguration(OrynEnumerationIndexedGet, nullptr,
            OrynEnumerationIndexedQuery, nullptr, OrynEnumerationIndices));
    target = object_template->NewInstance(owner).ToLocalChecked();
    auto initial_prototype = target->GetPrototype();
    CHECK(initial_prototype->IsObject());
    auto prototype = initial_prototype.As<v8::Object>();
    CHECK(prototype->HasOwnProperty(owner, v8_str("must_not_leak")).FromJust());
    CHECK(prototype->Get(owner, v8_str("must_not_leak")).ToLocalChecked()->IsTrue());
    v8::PropertyAttribute attributes = v8::None;
    CHECK(prototype->GetPropertyAttributes(
        owner, v8_str("must_not_leak"), &attributes).FromJust());
    CHECK_EQ(v8::None, attributes);
    CHECK(!target->HasOwnProperty(owner, v8_str("must_not_leak")).FromJust());

    // Same-prototype requests are allowed; a different prototype is rejected
    // only for the opted-in instance. The public API reports rejection as an
    // empty Maybe, so do not call FromJust on that expected negative result.
    CHECK(target->SetPrototype(owner, prototype).FromJust());
    auto replacement = v8::Object::New(isolate);
    if (opted_in) {
      CHECK(target->SetPrototype(owner, replacement).IsNothing());
      CHECK(target->GetPrototype()->StrictEquals(prototype));
    } else {
      CHECK(target->SetPrototype(owner, replacement).FromJust());
      CHECK(target->GetPrototype()->StrictEquals(replacement));
      CHECK(target->SetPrototype(owner, prototype).FromJust());
      CHECK(target->GetPrototype()->StrictEquals(prototype));
    }
    CHECK(target->Set(owner, v8_str("secret"), v8::True(isolate)).FromJust());
  }
  auto foreign = v8::Context::New(isolate);
  v8::Context::Scope entered(foreign);
  CHECK(foreign->Global()->Set(foreign, v8_str("target"), target).FromJust());
  CHECK(CompileRun("Object.keys(target).join(',') === '0'")->IsTrue());
  CHECK(CompileRun("Reflect.ownKeys(target).join(',') === '0,hidden'")->IsTrue());
  CHECK(CompileRun(opted_in
      ? "(()=>{let keys=[];for(const key in target)keys.push(key);return keys.join(',')==='0'})()"
      : "(()=>{let count=0;for(const key in target)count++;return count===0})()")->IsTrue());
  CHECK(CompileRun("(()=>{try{target.secret;return false}catch(e){return true}})()")->IsTrue());
}
}  // namespace
TEST(OrynCrossOriginForInUsesOnlyOptedInEnumerableOwnKeys) {
  CheckOrynCrossOriginEnumeration(true);
}
TEST(OrynUnrelatedAccessCheckedForInRemainsEmpty) {
  CheckOrynCrossOriginEnumeration(false);
}
