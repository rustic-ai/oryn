#![cfg(all(unix, feature = "v8-host", feature = "in-process-probe"))]
use oryn_common::{agent::Target, v3::SemanticAction};
use oryn_native::oil::NativeOilSession;
use oryn_native::runtime::{Browser, NativeAction, RuntimeError};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execution_rechecks_current_native_capability_not_old_reference() {
    let context = Browser::in_process_probe().new_context();
    let page = context.new_page();
    page.load_html(
        "https://actions.test/",
        "<span id=s>Plain</span><input id=i><button id=b>Button</button>",
    )
    .await
    .unwrap();
    let span = page
        .resolve_target(
            Target::Css {
                selector: "#s".into(),
            },
            None,
        )
        .await
        .unwrap()
        .selected
        .unwrap();
    assert!(matches!(
        page.execute(NativeAction {
            target: span,
            action: SemanticAction::Click,
            value: None
        })
        .await,
        Err(RuntimeError::UnsupportedAction { .. })
    ));
    page.evaluate("globalThis.clicks=0;globalThis.onNativeClick=()=>clicks++;s.addEventListener('click',onNativeClick)").await.unwrap();
    page.execute(NativeAction {
        target: span,
        action: SemanticAction::Click,
        value: None,
    })
    .await
    .unwrap();
    assert_eq!(page.evaluate("clicks").await.unwrap(), "1");
    page.evaluate("s.removeEventListener('click',onNativeClick)")
        .await
        .unwrap();
    assert!(matches!(
        page.execute(NativeAction {
            target: span,
            action: SemanticAction::Click,
            value: None
        })
        .await,
        Err(RuntimeError::UnsupportedAction { .. })
    ));
    let input = page
        .resolve_selector("#i", SemanticAction::Type)
        .await
        .unwrap();
    page.execute(NativeAction {
        target: input,
        action: SemanticAction::Type,
        value: Some("accepted".into()),
    })
    .await
    .unwrap();
    page.evaluate("i.readOnly=true").await.unwrap();
    assert!(
        page.execute(NativeAction {
            target: input,
            action: SemanticAction::Type,
            value: Some("blocked".into())
        })
        .await
        .is_err()
    );
    assert_eq!(page.evaluate("i.value").await.unwrap(), "accepted");
    page.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forced_frame_action_uses_child_capabilities_and_preserves_root() {
    let context = Browser::in_process_probe().new_context();
    let page = context.new_page();
    page.load_html("https://actions.test/frame",r#"<output id=status>root</output><iframe id=child srcdoc="<span id=plain>Plain</span><button id=hidden hidden onclick=&quot;document.getElementById('status').textContent='child'&quot;>Hidden action</button><button id=disabled disabled>Disabled</button><output id=status>waiting</output>"></iframe>"#).await.unwrap();
    let mut oil = NativeOilSession::new(page.clone());
    oil.execute("frame css(\"#child\")").await.unwrap();
    assert!(oil.execute("click css(\"#plain\") --force").await.is_err());
    assert!(
        oil.execute("click css(\"#disabled\") --force")
            .await
            .is_err()
    );
    oil.execute("click css(\"#hidden\") --force").await.unwrap();
    assert_eq!(page.evaluate("status.textContent").await.unwrap(), "root");
    assert_eq!(
        page.evaluate("child.contentDocument.getElementById('status').textContent")
            .await
            .unwrap(),
        "child"
    );
    page.close().await.unwrap();
}
