//! Selection.deleteFromDocument invokes the associated DOM Range algorithm.
//! It must not delete a rendered flat-tree interval (slots can reorder nodes).
use super::*;
pub(super) fn delete(state: &V8BindingState) -> Result<(), V8BindingError> {
    synchronize_selection_range(state)?;
    let Some(id) = state.selection_range.get() else {
        return Ok(());
    };
    let range: RangeRecord = state.dom.borrow().live_range(LiveRangeId(id))?.into();
    if range.start_container == range.end_container && range.start_offset == range.end_offset {
        return Ok(());
    }
    let collapse = range_contents_collapse(&state.dom.borrow(), &range)?;
    range_contents(
        &mut state.dom.borrow_mut(),
        &range,
        RangeContentsMode::Delete,
    )?;
    // Keep the script-held Range identity. Reactions run at the surrounding
    // generated CEReactions boundary, after these native endpoints are committed.
    let collapsed = RangeRecord {
        start_container: collapse.container,
        start_offset: collapse.offset,
        end_container: collapse.container,
        end_offset: collapse.offset,
    };
    state.ranges.borrow_mut().insert(id, collapsed);
    persist_live_range(state, id)?;
    state.selected_text_control.borrow_mut().take();
    synchronize_selection_range(state)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deletion_uses_dom_children_with_shadow_slots_and_preserves_associated_range() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><main></main></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, context, dom, RealmId(209), 1).unwrap();
        let source=v8::String::new(scope,r#"(()=>{
   const check=(x,m)=>{if(!x)throw Error(m)},main=document.querySelector('main'),s=getSelection();
   for(const slots of [false,true]){main.innerHTML='<p>abcdef</p><div><span>UnassignedText</span></div>';const host=main.lastChild,root=host.attachShadow({mode:'closed'});root.innerHTML=slots?'<b>InnerText</b><slot></slot>':'<b>InnerText</b>';const shadow=root.innerHTML,r=new Range();r.setStart(main.firstChild.firstChild,3);r.setEnd(host,1);s.removeAllRanges();s.addRange(r);s.deleteFromDocument();check(main.firstChild.textContent==='abc'&&host.childNodes.length===0,'DOM interval deleted');check(root.innerHTML===shadow,'shadow subtree remains intact');check(s.getRangeAt(0)===r&&r.collapsed,'same associated Range collapsed');}
   main.innerHTML='<div></div>';const root=main.firstChild.attachShadow({mode:'open'});root.textContent='abcdef';const r=new Range();r.setStart(root.firstChild,2);r.setEnd(root.firstChild,4);s.removeAllRanges();s.addRange(r);s.deleteFromDocument();check(root.textContent==='abef'&&r.startOffset===2&&r.collapsed,'same-shadow DOM range deletion');
   main.innerHTML='<details><summary>Summary</summary>Hidden</details>';const d=main.firstChild;r.selectNodeContents(d);s.removeAllRanges();s.addRange(r);s.deleteFromDocument();check(d.childNodes.length===0,'closed details rendering does not filter DOM deletion');
  })()"#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
}
