//! Native IDBVersionChangeEvent dictionary conversion and readonly backing.
use super::*;

fn key<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("IDB event private key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}

fn integer<'s>(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<'s, v8::Value>) -> Result<f64, V8BindingError> {
    let number = value.number_value(scope).ok_or(V8BindingError::JavaScriptException)?;
    if !number.is_finite() || number == 0.0 { return Ok(0.0); }
    // Ordinary WebIDL unsigned long long conversion, not EnforceRange.
    let converted = number.trunc().rem_euclid(18446744073709551616.0);
    Ok(if converted == 0.0 { 0.0 } else { converted })
}

pub(super) fn convert<'s>(scope: &mut v8::PinScope<'s, '_>, init: Option<v8::Local<'s, v8::Object>>) -> Result<(f64, Option<f64>), V8BindingError> {
    // Flattened EventInit members were read by the shared event constructor.
    // Remaining members are converted lexically: newVersion before oldVersion.
    let new = event_init_member(scope, init, "newVersion").ok_or(V8BindingError::JavaScriptException)?;
    let new = if new.is_null_or_undefined() { None } else { Some(integer(scope, new)?) };
    let old = event_init_member(scope, init, "oldVersion").ok_or(V8BindingError::JavaScriptException)?;
    let old = if old.is_undefined() { 0.0 } else { integer(scope, old)? };
    Ok((old, new))
}

pub(super) fn initialize<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>, values: (f64, Option<f64>)) -> Result<(), V8BindingError> {
    let old_key = key(scope, "oryn.idb.event.oldVersion")?;
    let old: v8::Local<v8::Value> = v8::Number::new(scope, values.0).into();
    let new_key = key(scope, "oryn.idb.event.newVersion")?;
    let new: v8::Local<v8::Value> = match values.1 { Some(n) => v8::Number::new(scope, n).into(), None => v8::null(scope).into() };
    if object.set_private(scope, old_key, old) != Some(true) || object.set_private(scope, new_key, new) != Some(true) {
        return Err(V8BindingError::Allocation("IDB event backing"));
    }
    Ok(())
}

pub(super) fn get<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>, member: &str) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if object.is_proxy() { return Err(V8BindingError::Unsupported { interface: "IDBVersionChangeEvent".into(), member: "TypeError".into() }); }
    let old_key = key(scope, "oryn.idb.event.oldVersion")?;
    if object.has_private(scope, old_key) != Some(true) { return Err(V8BindingError::Unsupported { interface: "IDBVersionChangeEvent".into(), member: "TypeError".into() }); }
    let field = if member == "oldVersion" { old_key } else { key(scope, "oryn.idb.event.newVersion")? };
    object.get_private(scope, field).ok_or(V8BindingError::Allocation("IDB event backing"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_idb_event_dictionary_and_promise_receiver_contract() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap()));
        let _state = install_generated_bindings(scope, context, dom, RealmId(913), 1).unwrap();
        let source = v8::String::new(scope, r#"
          const eq=(a,b)=>{if(a!==b)throw Error(`${a} != ${b}`)};
          eq(IDBVersionChangeEvent.length,1);
          for(const init of [true,1,'x',Symbol()]){let invalid=false;try{new IDBVersionChangeEvent('v',init)}catch(x){invalid=x instanceof TypeError}eq(invalid,true)}
          eq(new IDBVersionChangeEvent('v',null).oldVersion,0);
          let missing=false;try{new IDBVersionChangeEvent()}catch(x){missing=x instanceof TypeError}eq(missing,true);
          const defaults=new IDBVersionChangeEvent('versionchange');
          eq(defaults.oldVersion,0);eq(defaults.newVersion,null);
          eq(Object.hasOwn(defaults,'oldVersion'),false);
          eq(Object.hasOwn(defaults,'newVersion'),false);
          let reads=[];
          const e=new IDBVersionChangeEvent('v',{
            get bubbles(){reads.push('bubbles');return true},
            get cancelable(){reads.push('cancelable');return false},
            get composed(){reads.push('composed');return true},
            get newVersion(){reads.push('newVersion');return {valueOf(){reads.push('new-number');return 5.9}}},
            get oldVersion(){reads.push('oldVersion');return -2.5}
          });
          eq(reads.join(','),'bubbles,cancelable,composed,newVersion,new-number,oldVersion');
          eq(e.newVersion,5);eq(e.oldVersion,18446744073709551616);
          const old=Object.getOwnPropertyDescriptor(IDBVersionChangeEvent.prototype,'oldVersion');
          eq(typeof old.get,'function');eq(old.set,undefined);
          let wrong=false;try{old.get.call({})}catch(x){wrong=x instanceof TypeError}eq(wrong,true);
          const marker={};reads=[];
          try{new IDBVersionChangeEvent('v',{get newVersion(){throw marker},get oldVersion(){reads.push('old')}})}catch(x){eq(x,marker)}
          eq(reads.length,0);
          const nested=new IDBVersionChangeEvent('v',{get newVersion(){return new IDBVersionChangeEvent('nested',{oldVersion:7}).oldVersion}});
          eq(nested.newVersion,7);
          globalThis.receiverResult='pending';
          let p;try{p=IDBFactory.prototype.databases.call({})}catch(x){throw Error('synchronous receiver throw')}
          eq(p instanceof Promise,true);
          p.then(()=>receiverResult='wrong',e=>receiverResult=e instanceof TypeError?'ok':'wrong');
          globalThis.sheetResult='pending';
          const q=CSSStyleSheet.prototype.replace.call({},'a{}');
          eq(q instanceof Promise,true);
          q.then(()=>sheetResult='wrong',e=>sheetResult=e instanceof TypeError?'ok':'wrong');
          'ok';
        "#).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(script.run(scope).unwrap().to_rust_string_lossy(scope), "ok");
        scope.perform_microtask_checkpoint();
        let source = v8::String::new(scope, "receiverResult").unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(script.run(scope).unwrap().to_rust_string_lossy(scope), "ok");
        let source = v8::String::new(scope, "sheetResult").unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(script.run(scope).unwrap().to_rust_string_lossy(scope), "ok");
    }
}
