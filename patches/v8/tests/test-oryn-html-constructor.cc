// Copyright 2026 the V8 project authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

#include <initializer_list>
#include <string>

#include "include/v8.h"
#include "test/cctest/cctest.h"

namespace {
void OrynHtmlConstructor(const v8::FunctionCallbackInfo<v8::Value>& args) {
  auto* isolate = args.GetIsolate();
  auto context = isolate->GetCurrentContext();
  CHECK(args.IsConstructCall());
  if (!args.Data()->IsTrue()) {
    isolate->ThrowException(v8::Exception::TypeError(v8_str("unregistered")));
    return;
  }
  v8::Local<v8::Value> prototype;
  if (!args.NewTarget().As<v8::Object>()
           ->Get(context, v8_str("prototype")).ToLocal(&prototype)) return;
  CHECK(prototype->IsObject());
  CHECK(args.This()->SetPrototype(context, prototype).FromJust());
  args.GetReturnValue().Set(args.This());
}
}

TEST(OrynHtmlConstructorDefersPrototypeUntilAdmission) {
  v8::HandleScope handles(CcTest::isolate());
  LocalContext context;
  auto* isolate = context.isolate();
  for (const char* name : {"Accepted", "Rejected", "Ordinary"}) {
    bool accepted = std::string(name) == "Accepted";
    auto native = v8::FunctionTemplate::New(
        isolate, OrynHtmlConstructor, v8::Boolean::New(isolate, accepted));
    native->InstanceTemplate()->SetInternalFieldCount(2);
    if (std::string(name) != "Ordinary") native->SetOrynDeferredPrototype(true);
    CHECK(context->Global()->Set(context.local(), v8_str(name),
          native->GetFunction(context.local()).ToLocalChecked()).FromJust());
  }
  CHECK(CompileRun(R"JS(
    (() => {
      let gets = 0;
      class Derived extends Accepted {}
      const target = new Proxy(Derived, {
        get(object, key, receiver) {
          if (key === 'prototype') ++gets;
          return Reflect.get(object, key, receiver);
        }
      });
      try { Reflect.construct(Rejected, [], target); return false; }
      catch (error) { if (!(error instanceof TypeError)) return false; }
      if (gets !== 0) return false;
      const result = Reflect.construct(Accepted, [], target);
      if (gets !== 1 || !(result instanceof Derived)) return false;
      gets = 0;
      try { Reflect.construct(Ordinary, [], target); return false; }
      catch (error) { if (!(error instanceof TypeError)) return false; }
      return gets === 1;
    })()
  )JS")->IsTrue());
}
