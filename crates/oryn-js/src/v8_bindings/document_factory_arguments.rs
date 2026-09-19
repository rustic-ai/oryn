//! Required WebIDL arguments are checked before converting any supplied value.
use super::*;

pub(super) fn require(
    args: &v8::FunctionCallbackArguments,
    count: i32,
) -> Result<(), V8BindingError> {
    if args.length() < count {
        Err(V8BindingError::Unsupported {
            interface: "Document".into(),
            member: "TypeError".into(),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_factory_arguments_do_not_convert_and_undefined_remains_a_value() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let code = v8::String::new(scope, r#"(()=>{
          const check=ok=>{if(!ok)throw Error('factory conversion')};
          const throws=(fn,name)=>{try{fn()}catch(e){check(e.name===name);return}throw Error('missing error')};
          for(const doc of [document,document.implementation.createDocument(null,'root')]) {
            for(const method of ['createElement','createAttribute','createTextNode','createComment','createCDATASection','createProcessingInstruction','createElementNS','createAttributeNS'])throws(()=>doc[method](),'TypeError');
            for(const method of ['createElementNS','createAttributeNS','createProcessingInstruction']) {
              let converted=false;
              throws(()=>doc[method]({toString(){converted=true;throw Error('conversion must not run')}}),'TypeError');
              check(!converted);
            }
            check(doc.createElement(undefined).localName==='undefined');
            check(doc.createAttribute(undefined).name==='undefined');
            check(doc.createElementNS(null,undefined).localName==='undefined');
            check(doc.createAttributeNS(null,undefined).localName==='undefined');
            for(const method of ['createTextNode','createComment']) {
              check(doc[method](undefined).data==='undefined');
              check(doc[method](null).data==='null');
              check(doc[method]('\ud800').data==='\ud800');
              check(doc[method]('').ownerDocument===doc);
            }
            const order=[];
            const attr=doc.createAttributeNS({toString(){order.push('namespace');return 'urn:test'}},{toString(){order.push('name');return 'p:Name'}});
            check(order.join()==='namespace,name'&&attr.prefix==='p'&&attr.localName==='Name'&&attr.namespaceURI==='urn:test'&&attr.ownerDocument===doc);
          }
          return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}
