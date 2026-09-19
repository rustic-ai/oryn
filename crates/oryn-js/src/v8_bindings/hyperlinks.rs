//! Hyperlink URL components use the element's owner document and its encoding.
use super::*;

pub(super) fn handles(interface: &str, member: &str) -> bool {
    matches!(interface, "HTMLAnchorElement" | "HTMLAreaElement")
        && matches!(
            member,
            "href"
                | "origin"
                | "protocol"
                | "username"
                | "password"
                | "host"
                | "hostname"
                | "port"
                | "pathname"
                | "search"
                | "hash"
        )
}

pub(super) fn document_encoding(
    state: &V8BindingState,
    document: NodeId,
) -> Result<&'static encoding_rs::Encoding, V8BindingError> {
    let dom = state.dom.borrow();
    Ok(dom
        .arena()
        .get(document)?
        .document_metadata
        .as_ref()
        .and_then(|metadata| encoding_rs::Encoding::for_label(metadata.encoding.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8)
        .output_encoding())
}

pub(super) fn parse(
    input: &str,
    base: Option<&Url>,
    encoding: &'static encoding_rs::Encoding,
) -> Result<Url, url::ParseError> {
    oryn_dom::document_url::parse(input, base, encoding)
}

pub(super) fn document_base(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    document: NodeId,
) -> Result<(String, &'static encoding_rs::Encoding), V8BindingError> {
    let encoding = document_encoding(state, document)?;
    let base = if document == state.document_root() {
        document_base_url(scope, state, true)?
    } else {
        retained_document_base_url(state, document)?
    };
    Ok((base, encoding))
}

fn resolved(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    node: NodeId,
) -> Result<(Option<String>, Option<Url>, &'static encoding_rs::Encoding), V8BindingError> {
    let document = owner_document_for_node(state, node)?;
    let (base, encoding) = document_base(scope, state, document)?;
    let raw = state
        .dom
        .borrow()
        .attribute_in_null_namespace(node, "href")?
        .map(str::to_owned);
    let url = raw
        .as_deref()
        .and_then(|raw| parse(raw, Url::parse(&base).ok().as_ref(), encoding).ok());
    Ok((raw, url, encoding))
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    node: NodeId,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let (raw, url, _) = resolved(scope, state, node)?;
    let Some(url) = url else {
        return v8_string(
            scope,
            if member == "href" {
                raw.as_deref().unwrap_or("")
            } else if member == "protocol" {
                ":"
            } else {
                ""
            },
        );
    };
    let value = match member {
        "href" => url::quirks::href(&url),
        "origin" => return v8_string(scope, &url::quirks::origin(&url)),
        "protocol" => url::quirks::protocol(&url),
        "username" => url::quirks::username(&url),
        "password" => url::quirks::password(&url),
        "host" => url::quirks::host(&url),
        "hostname" => url::quirks::hostname(&url),
        "port" => url::quirks::port(&url),
        "pathname" => url::quirks::pathname(&url),
        "search" => url::quirks::search(&url),
        "hash" => url::quirks::hash(&url),
        _ => unreachable!(),
    };
    v8_string(scope, value)
}

pub(super) fn set(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    node: NodeId,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let input = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    if member == "href" {
        state
            .dom
            .borrow_mut()
            .set_attribute_ns(node, None, "href", input)?;
        return Ok(());
    }
    let (_, Some(mut url), encoding) = resolved(scope, state, node)? else {
        return Ok(());
    };
    match member {
        "protocol" => {
            let _ = url::quirks::set_protocol(&mut url, &input);
        }
        "username" => {
            let _ = url::quirks::set_username(&mut url, &input);
        }
        "password" => {
            let _ = url::quirks::set_password(&mut url, &input);
        }
        "host" => {
            let _ = url::quirks::set_host(&mut url, &input);
        }
        "hostname" => {
            let _ = url::quirks::set_hostname(&mut url, &input);
        }
        "port" => {
            let _ = url::quirks::set_port(&mut url, &input);
        }
        "pathname" => url::quirks::set_pathname(&mut url, &input),
        "search" => {
            let fragment = url.fragment().map(str::to_owned);
            url.set_fragment(None);
            url.set_query(None);
            if !input.is_empty() {
                let query = input.strip_prefix('?').unwrap_or(&input);
                // '#' is data in a search setter, never a fragment delimiter.
                let query = query.replace('#', "%23");
                if let Ok(updated) = parse(&format!("{url}?{query}"), None, encoding) {
                    url = updated;
                }
            }
            url.set_fragment(fragment.as_deref());
        }
        "hash" => url::quirks::set_hash(&mut url, &input),
        _ => return Ok(()),
    }
    state
        .dom
        .borrow_mut()
        .set_attribute_ns(node, None, "href", url.as_str())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn iso2022jp_query_escapes_replacement_references_for_disallowed_controls() {
        for (input, expected) in [
            (
                "\u{0e}\u{0f}\u{1b}x",
                "?%26%2365533%3B%26%2365533%3B%26%2365533%3Bx",
            ),
            (
                "‾\u{0e}\u{0f}\u{1b}x",
                "?%1B(J~%26%2365533%3B%26%2365533%3B%26%2365533%3Bx%1B(B",
            ),
        ] {
            let url = parse(
                &format!("https://example.test/?{input}"),
                None,
                encoding_rs::ISO_2022_JP,
            )
            .unwrap();
            assert_eq!(url::quirks::search(&url), expected);
        }
    }

    #[test]
    fn reflected_resource_urls_share_document_encoding_and_ping_remains_a_string() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let document = dom.borrow().document();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/path").unwrap();
        for (encoding, suffix) in [
            ("windows-1251", "%26%23255%3B"),
            ("windows-1252", "%FF"),
            ("UTF-8", "%C3%BF"),
        ] {
            state
                .dom
                .borrow_mut()
                .arena_mut()
                .get_mut(document)
                .unwrap()
                .document_metadata
                .as_mut()
                .unwrap()
                .encoding = encoding.into();
            let source = format!(
                r#"(()=>{{const expected='?{suffix}';
              for(const [tag,attribute,property] of [['iframe','src','src'],['a','href','href'],['area','href','href'],['base','href','href'],['link','href','href'],['img','src','src'],['embed','src','src'],['object','data','data'],['track','src','src'],['video','src','src'],['audio','src','src'],['input','src','src'],['form','action','action'],['input','formaction','formAction'],['button','formaction','formAction'],['script','src','src']]){{const element=document.createElement(tag);element.setAttribute(attribute,'?ÿ');if(!element[property].endsWith(expected))throw Error(tag+': '+element[property]);element[property]='?ÿ';if(element.getAttribute(attribute)!=='?ÿ')throw Error('setter '+tag);}}
              for(const tag of ['a','area']){{const element=document.createElement(tag);element.ping='?ÿ ?ÿ';if(element.ping!=='?ÿ ?ÿ')throw Error('ping');}}
              const base=document.createElement('base');base.href='?ÿ';document.head.appendChild(base);const image=document.createElement('img');image.src='#suffix';if(!image.src.includes(expected+'#suffix'))throw Error('encoded base');base.remove();return 'ok';}})()"#
            );
            v8::tc_scope!(let inner,scope);
            let source = v8::String::new(inner, &source).unwrap();
            let result = v8::Script::compile(inner, source, None).unwrap().run(inner);
            if let Some(error) = inner.exception() {
                panic!("{}", error.to_rust_string_lossy(inner));
            }
            assert_eq!(result.unwrap().to_rust_string_lossy(inner), "ok");
        }
    }

    #[test]
    fn hyperlink_components_use_legacy_encoding_and_live_native_base() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let document = dom.borrow().document();
        dom.borrow_mut()
            .arena_mut()
            .get_mut(document)
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .encoding = "Big5".into();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/start").unwrap();
        v8::tc_scope!(let scope, scope);
        let source = v8::String::new(scope, r#"
          const eq=(a,b)=>{if(a!==b)throw Error(JSON.stringify([a,b]))};
          for (const tag of ['a','area']) {
            const a=document.createElement(tag); eq(a.href,'');eq(a.protocol,':');
            a.href='/path?X一X';eq(a.search,'?X%A4@X');eq(a.pathname,'/path');
            a.search='?☃#end';eq(a.search,'?%26%239731%3B%23end');
            a.hash='frag';a.host='other.test:8443';a.username='user';a.password='pass';
            eq(a.origin,'https://other.test:8443');eq(a.host,'other.test:8443');eq(a.port,'8443');eq(a.hash,'#frag');
            a.search='';eq(a.search,'');eq(a.hash,'#frag');
            a.href='relative';const base=document.createElement('base');base.href='https://base.test/dir/';document.head.appendChild(base);
            eq(a.href,'https://base.test/dir/relative');base.remove();eq(a.href,'https://example.test/relative');
            const other=document.implementation.createHTMLDocument('other');
            other.head.innerHTML='<base href="https://owner.test/">';other.adoptNode(a);eq(a.href,'https://owner.test/relative');
            a.href='https://owner.test/?一';eq(a.search,'?%E4%B8%80');
          }
          'ok'
        "#).unwrap();
        let result = v8::Script::compile(scope, source, None).unwrap().run(scope);
        if let Some(error) = scope.exception() {
            panic!("{}", error.to_rust_string_lossy(scope));
        }
        assert_eq!(result.unwrap().to_rust_string_lossy(scope), "ok");
    }
    #[test]
    fn form_submissions_encode_text_and_preserve_uploaded_bytes() {
        super::super::tests::initialize_v8();
        for (method, enctype) in [
            ("get", "application/x-www-form-urlencoded"),
            ("post", "application/x-www-form-urlencoded"),
            ("post", "text/plain"),
            ("post", "multipart/form-data"),
        ] {
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            let document = dom.borrow().document();
            dom.borrow_mut()
                .arena_mut()
                .get_mut(document)
                .unwrap()
                .document_metadata
                .as_mut()
                .unwrap()
                .encoding = "Big5".into();
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope, &mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
            *state.location_url.borrow_mut() = Url::parse("https://example.test/").unwrap();
            v8::tc_scope!(let scope, scope);
            let source = format!(
                r#"
                const form=document.createElement('form');document.body.append(form);
                form.action='https://example.test/post';form.method='{method}';form.enctype='{enctype}';
                form.innerHTML='<textarea name="field">一☃</textarea><input type="hidden" name="_charset_">';
                if(new FormData(form).get('_charset_')!=='UTF-8')throw Error('FormData constructor encoding');
                form.onformdata=e=>{{
                    if(e.formData.get('_charset_')!=='Big5')throw Error('submission charset');
                    e.formData.append('upload',new Blob([new Uint8Array([255,0,1])]),'一.bin');
                }};
                form.submit();'ok'
            "#
            );
            let source = v8::String::new(scope, &source).unwrap();
            let result = v8::Script::compile(scope, source, None).unwrap().run(scope);
            if let Some(error) = scope.exception() {
                panic!("{}", error.to_rust_string_lossy(scope));
            }
            assert_eq!(result.unwrap().to_rust_string_lossy(scope), "ok");
            let actions = state.take_pending_default_actions();
            assert_eq!(actions.len(), 1);
            let PendingDefaultAction::Submit(submission) = &actions[0] else {
                panic!("expected form submission")
            };
            if method == "get" {
                assert!(submission.navigation.post.is_none());
                assert_eq!(
                    submission.navigation.url,
                    "https://example.test/post?field=%A4%40%26%239731%3B&_charset_=Big5&upload=%A4%40.bin"
                );
                continue;
            }
            let post = submission.navigation.post.as_ref().unwrap();
            if enctype == "application/x-www-form-urlencoded" {
                assert_eq!(
                    String::from_utf8(post.bytes.clone()).unwrap(),
                    "field=%A4%40%26%239731%3B&_charset_=Big5&upload=%A4%40.bin"
                );
            } else if enctype == "text/plain" {
                assert!(
                    post.bytes
                        .windows(17)
                        .any(|bytes| bytes == b"field=\xa4@&#9731;\r\n"),
                    "{:?}",
                    post.bytes
                );
            } else {
                assert!(post.bytes.windows(9).any(|bytes| bytes == b"\xa4@&#9731;"));
                assert!(post.bytes.windows(3).any(|bytes| bytes == [255, 0, 1]));
                assert!(post.bytes.windows(6).any(|bytes| bytes == b"\xa4@.bin"));
            }
            let source = v8::String::new(scope, "form.onformdata=e=>{if(e.formData.get('_charset_')!=='UTF-8')throw Error('acceptCharset override')};form.acceptCharset='invalid-label utf-8 big5';form.method='get';form.submit();'ok'").unwrap();
            let result = v8::Script::compile(scope, source, None).unwrap().run(scope);
            if let Some(error) = scope.exception() {
                panic!("{}", error.to_rust_string_lossy(scope));
            }
            assert!(result.is_some());
            let overridden = state.take_pending_default_actions();
            let PendingDefaultAction::Submit(submission) = &overridden[0] else {
                panic!("expected overridden submission")
            };
            assert_eq!(
                submission.navigation.url,
                "https://example.test/post?field=%E4%B8%80%E2%98%83&_charset_=UTF-8"
            );
        }
    }
}
