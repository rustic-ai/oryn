//! Associated internals brands must not be confused with their host node.
use super::*;
#[test]
fn native_internals_brand_rejects_host_and_other_associated_wrappers() {
    tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let dom = Rc::new(RefCell::new(
        oryn_dom::html::parse("<body></body>")
            .into_runtime()
            .unwrap(),
    ));
    let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
    assert!(test_eval::eval(scope,r#"(()=>{
      customElements.define('x-brand',class extends HTMLElement{static formAssociated=true});
      const host=document.createElement('x-brand'),internals=host.attachInternals();
      const root=host.attachShadow({mode:'closed'});document.body.append(host);
      const get=Object.getOwnPropertyDescriptor(ElementInternals.prototype,'shadowRoot').get;
      if(get.call(internals)!==root||internals.shadowRoot!==root||host.shadowRoot!==null)return false;
      for(const receiver of [host,host.classList,host.style,{},root]){
        let failed=false;try{get.call(receiver)}catch(error){failed=error instanceof TypeError}
        if(!failed)return false;
        failed=false;try{ElementInternals.prototype.checkValidity.call(receiver)}catch(error){failed=error instanceof TypeError}
        if(!failed)return false;
      }
      host.remove();return internals.shadowRoot===root;
    })()"#).is_true());
}
