//! XML Name validation for the processing-instruction factory. HTML element
//! and attribute factory names deliberately use different validation rules.
use super::*;

fn name_start(c: char) -> bool {
    matches!(c, ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}'
        | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}'
        | '\u{200c}'..='\u{200d}' | '\u{2070}'..='\u{218f}'
        | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}'
        | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}'
        | '\u{10000}'..='\u{effff}')
}

fn valid_target(target: &str) -> bool {
    // DOMString storage is lossless WTF-16. Validate its decoded scalar values,
    // never the private storage marker; unpaired surrogates are not XML names.
    let Ok(target) = String::from_utf16(&oryn_dom::dom_string::units(target)) else {
        return false;
    };
    let mut chars = target.chars();
    chars.next().is_some_and(name_start) && chars.all(|c| {
        name_start(c) || matches!(c, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
    })
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    document: NodeId,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let error = |member: &str| V8BindingError::Unsupported {
        interface: "Document".into(),
        member: member.into(),
    };
    document_factory_arguments::require(args, 2)?;
    // Complete WebIDL conversion before algorithm validation. In particular a
    // throwing data conversion wins over an invalid already-converted target.
    let target = required_string(scope, args, 0)?;
    let data = required_string(scope, args, 1)?;
    if !valid_target(&target)
        || oryn_dom::dom_string::units(&data)
            .windows(2)
            .any(|units| units == [63, 62])
    {
        return Err(error("InvalidCharacterError"));
    }
    let created = state
        .dom
        .borrow_mut()
        .create_processing_instruction(target, data);
    state
        .dom
        .borrow_mut()
        .set_node_document(created, document)?;
    Ok(Some(wrap_node(scope, state, created)?.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xml_name_boundaries_are_distinct_from_html_names() {
        for target in [
            "xml:fail",
            "A\u{b7}A",
            "a0",
            ":",
            "xml",
            "\u{10000}",
            "\u{effff}",
        ] {
            assert!(valid_target(target), "{target:?}");
        }
        for target in [
            "",
            "\u{b7}A",
            "\u{d7}A",
            "A\u{d7}",
            "\\A",
            "\u{c}",
            "0",
            "\u{37e}",
            "\u{ffff}",
            "\u{f0000}",
        ] {
            assert!(!valid_target(target), "{target:?}");
        }
        assert!(!valid_target(&oryn_dom::dom_string::from_units(&[0xd800])));
    }

    #[test]
    fn factory_validates_after_conversion_without_changing_owner_or_data() {
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
          const check=(ok)=>{if(!ok)throw Error('processing instruction')};
          const throws=(name,fn)=>{try{fn()}catch(e){check(e.name===name);return}throw Error('missing exception')};
          for(const doc of [document,document.implementation.createDocument(null,'root')]) {
            for(const args of [[],['a']])throws('TypeError',()=>doc.createProcessingInstruction(...args));
            for(const target of ['',0,'\u00b7A','\u00d7A','A\u00d7','\\A','\f','\ud800'])throws('InvalidCharacterError',()=>doc.createProcessingInstruction(target,'x'));
            for(const data of ['?>','\ud800?>','?>\udfff'])throws('InvalidCharacterError',()=>doc.createProcessingInstruction('A',data));
            for(const target of ['xml:fail','A\u00b7A','a0','xml']) {
              const pi=doc.createProcessingInstruction(target,'\ud800');
              check(pi instanceof ProcessingInstruction&&pi instanceof CharacterData&&!(pi instanceof Comment));
              check(pi.ownerDocument===doc&&pi.target===target&&pi.data==='\ud800'&&pi.parentNode===null);
            }
            const order=[],sentinel={};let caught;
            try{doc.createProcessingInstruction({toString(){order.push('target');return '0'}},{toString(){order.push('data');throw sentinel}})}catch(e){caught=e}
            check(caught===sentinel&&order.join()==='target,data');
            check(doc.createProcessingInstruction(undefined,undefined).data==='undefined');
          }
          const xml=document.implementation.createDocument(null,'root');
          for(const data of [']]>', '\ud800]]>', ']]>\udfff'])throws('InvalidCharacterError',()=>xml.createCDATASection(data));
          check(xml.createCDATASection('\ud800').data==='\ud800');
          check(xml.createCDATASection(undefined) instanceof CDATASection);
          throws('TypeError',()=>xml.createCDATASection());
          throws('TypeError',()=>document.createCDATASection());
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
