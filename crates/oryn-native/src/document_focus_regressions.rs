//! Real document bindings and parent-owned foreground selection.
use super::*;

#[tokio::test]
async fn document_has_focus_tracks_foreground_visibility_and_script_activation() {
    let context = Browser::native_test_with_config(BrowserConfig::default()).new_context();
    let first = context.new_page();
    let second = context.new_page();
    first
        .load_html("https://focus.test/one", "<input id=field>")
        .await
        .unwrap();
    second
        .load_html("https://focus.test/two", "<p>Second</p>")
        .await
        .unwrap();
    context.activate_page(first.page_id()).unwrap();
    assert_eq!(
        first.evaluate("String(document.hasFocus())").await.unwrap(),
        "true"
    );
    assert_eq!(
        second
            .evaluate("String(document.hasFocus())")
            .await
            .unwrap(),
        "false"
    );
    context.activate_page(second.page_id()).unwrap();
    assert_eq!(
        first.evaluate("String(document.hasFocus())").await.unwrap(),
        "false"
    );
    assert_eq!(
        second
            .evaluate("String(document.hasFocus())")
            .await
            .unwrap(),
        "true"
    );
    first.evaluate("window.focus()").await.unwrap();
    assert_eq!(
        first.evaluate("String(document.hasFocus())").await.unwrap(),
        "true"
    );
    assert_eq!(
        second
            .evaluate("String(document.hasFocus())")
            .await
            .unwrap(),
        "false"
    );
    context
        .webdriver_set_visibility(first.page_id(), true)
        .unwrap();
    assert_eq!(
        first.evaluate("String(document.hasFocus())").await.unwrap(),
        "false"
    );
    context
        .webdriver_set_visibility(first.page_id(), false)
        .unwrap();
    assert_eq!(
        first.evaluate("String(document.hasFocus())").await.unwrap(),
        "true"
    );
    first.evaluate("globalThis.oldFrame=document.createElement('iframe');document.body.append(oldFrame);globalThis.oldWindow=oldFrame.contentWindow;oldFrame.remove()").await.unwrap();
    context.activate_page(second.page_id()).unwrap();
    first.evaluate("oldWindow.focus()").await.unwrap();
    assert_eq!(context.active_page().unwrap().page_id(), second.page_id());
    assert_eq!(
        second
            .evaluate("String(document.hasFocus())")
            .await
            .unwrap(),
        "true"
    );
    first.close().await.unwrap();
    second.close().await.unwrap();
}

#[tokio::test]
async fn document_has_focus_nested_siblings_and_detached_receivers() {
    let context = Browser::native_test_with_config(BrowserConfig::default()).new_context();
    let page = context.new_page();
    page.load_html("https://focus.test/frames", r#"<iframe id=a srcdoc="<input id=x>"></iframe><iframe id=b srcdoc="<input id=y>"></iframe>"#).await.unwrap();
    context.activate_page(page.page_id()).unwrap();
    assert_eq!(page.evaluate(r#"globalThis.a=document.getElementById('a');globalThis.b=document.getElementById('b'); a.contentWindow.focus(); [document.hasFocus(),a.contentDocument.hasFocus(),b.contentDocument.hasFocus()].join('|')"#).await.unwrap(), "true|true|false");
    assert_eq!(page.evaluate(r#"b.contentWindow.focus(); [document.hasFocus(),a.contentDocument.hasFocus(),b.contentDocument.hasFocus()].join('|')"#).await.unwrap(), "true|false|true");
    assert_eq!(page.evaluate(r#"globalThis.oldFocusDoc=b.contentDocument; b.remove(); String(Document.prototype.hasFocus.call(oldFocusDoc))"#).await.unwrap(), "false");
    assert_eq!(page.evaluate(r#"String(Document.prototype.hasFocus.call(document.implementation.createHTMLDocument('inert')))"#).await.unwrap(), "false");
    assert_eq!(
        page.evaluate(r#"try {Document.prototype.hasFocus.call({}); 'bad'} catch(e) {e.name}"#)
            .await
            .unwrap(),
        "TypeError"
    );
    page.close().await.unwrap();
}

#[test]
fn document_focus_parent_rejects_parked_and_retired_authority() {
    use oryn_js::{
        message_wire::{MessageRequest, WindowTarget},
        platform_control::{Request, Response},
        worker_wire::Creator,
    };
    let context = Browser::in_process_with_config(BrowserConfig::default()).new_context();
    let page = context.new_page();
    let id = page.page_id();
    context
        .install_document_security(id, "https://focus.test/one".into(), &[])
        .unwrap();
    context
        .handle_message_wire(
            id,
            MessageRequest::RegisterRootDocument {
                document_generation: 1,
                stream_epoch: 0,
            },
        )
        .unwrap();
    context.activate_page(id).unwrap();
    let creator = Creator::Window {
        target: WindowTarget::Root,
        document_generation: 1,
        stream_epoch: 0,
    };
    assert!(matches!(
        context
            .native_platform_query(id, creator.clone(), Request::DocumentFocus)
            .unwrap(),
        Response::DocumentFocus { focused: true }
    ));
    let token = context
        .inner
        .group
        .messages
        .lock()
        .unwrap()
        .park_document_authority(id.0, 1, 0)
        .unwrap();
    assert!(matches!(
        context
            .native_platform_query(id, creator.clone(), Request::DocumentFocus)
            .unwrap(),
        Response::WindowSuspended
    ));
    context
        .inner
        .group
        .messages
        .lock()
        .unwrap()
        .discard_parked_document(id.0, &token)
        .unwrap();
    assert!(matches!(
        context
            .native_platform_query(id, creator, Request::DocumentFocus)
            .unwrap(),
        Response::WindowRetired
    ));
}
