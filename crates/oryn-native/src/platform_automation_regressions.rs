//! Parent admission regression sources for window-scoped automation controls.
use super::*;
use oryn_js::platform_control::{
    PermissionDescriptor, PermissionState, PositionError, RegistrationMode, Request, Response,
};
fn position(context: &BrowserContext, page: PageId) -> serde_json::Value {
    serde_json::to_value(
        context
            .inner
            .platform_control
            .lock()
            .unwrap()
            .query(page, "https://platform.test", Request::Geolocation)
            .unwrap(),
    )
    .unwrap()
}
#[tokio::test]
async fn retired_platform_context_rejects_entire_location_batch_and_cannot_restore_overrides() {
    let context = Browser::in_process_probe().new_context_with_options(ContextOptions::default());
    let live = context.new_page();
    let closed = context.new_page();
    context
        .webdriver_set_permission(
            "https://platform.test",
            PermissionDescriptor {
                name: "geolocation".into(),
                sysex: None,
                pan_tilt_zoom: None,
                allow_without_gesture: None,
            },
            PermissionState::Granted,
        )
        .unwrap();
    context
        .webdriver_set_geolocation(
            live.page_id(),
            Some(Err(PositionError {
                code: 2,
                message: "original".into(),
            })),
        )
        .unwrap();
    closed.close().await.unwrap();
    let before = position(&context, live.page_id());
    let selected = std::collections::BTreeSet::from([live.page_id(), closed.page_id()]);
    assert!(matches!(
        context.webdriver_set_geolocation_many(
            &selected,
            Some(Err(PositionError {
                code: 2,
                message: "replacement".into()
            }))
        ),
        Err(RuntimeError::Closed)
    ));
    assert_eq!(position(&context, live.page_id()), before);
    assert!(matches!(
        context.webdriver_set_geolocation_many(&selected, None),
        Err(RuntimeError::Closed)
    ));
    assert!(matches!(
        context.webdriver_set_visibility(closed.page_id(), true),
        Err(RuntimeError::Closed)
    ));
    assert!(matches!(
        context.webdriver_set_registration_mode(closed.page_id(), RegistrationMode::Accept),
        Err(RuntimeError::Closed)
    ));
    assert_eq!(position(&context, live.page_id()), before);
    assert!(matches!(
        context
            .inner
            .platform_control
            .lock()
            .unwrap()
            .query(
                closed.page_id(),
                "https://platform.test",
                Request::WindowState
            )
            .unwrap(),
        Response::WindowState { hidden: false, .. }
    ));
    live.close().await.unwrap();
}
#[tokio::test]
async fn crashed_platform_context_and_empty_batch_fail_before_mutation() {
    let context = Browser::in_process_probe().new_context_with_options(ContextOptions::default());
    let page = context.new_page();
    let before = position(&context, page.page_id());
    assert!(
        context
            .webdriver_set_geolocation_many(&std::collections::BTreeSet::new(), None)
            .is_err()
    );
    assert_eq!(position(&context, page.page_id()), before);
    context
        .inner
        .group
        .crash(page.page_id(), "regression retirement".into());
    assert!(matches!(
        context.webdriver_set_visibility(page.page_id(), true),
        Err(RuntimeError::Closed)
    ));
    assert!(matches!(
        context.webdriver_set_registration_mode(page.page_id(), RegistrationMode::Reject),
        Err(RuntimeError::Closed)
    ));
    assert_eq!(position(&context, page.page_id()), before);
    page.close().await.unwrap();
}

#[tokio::test]
async fn cookie_delete_uses_current_locked_url_and_rejects_retirement_before_store_effect() {
    use oryn_js::message_wire::MessageRequest;
    let context = Browser::in_process_probe().new_context_with_options(ContextOptions::default());
    let page = context.new_page();
    let id = page.page_id();
    context
        .install_document_security(id, "https://cookies.test/first/path".into(), &[])
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
    let first = Url::parse("https://cookies.test/first/path").unwrap();
    let second = Url::parse("https://cookies.test/second/path").unwrap();
    let store = context.inner.parent_cookies.as_ref().unwrap();
    store
        .store_response(
            &first,
            &crate::cookies::CookieRequestContext::navigation(Some(first.to_string())),
            [
                "first=yes; Secure; HttpOnly; Path=/first".into(),
                "second=yes; Secure; HttpOnly; Path=/second".into(),
            ],
        )
        .unwrap();
    let mut snapshot = crate::host::webdriver_script::AutomationRealmSnapshot {
        realm: oryn_js::realm::RealmId(0),
        parent_realm: None,
        frame: None,
        frame_generation: None,
        frame_token: None,
        document_root: oryn_dom::DomRuntime::new().document(),
        document_generation: 1,
        stream_epoch: 0,
        document_authority_epoch: 0,
        url: "https://forged.test".into(),
        live: true,
    };
    snapshot.parent_realm = Some(oryn_js::realm::RealmId(7));
    snapshot.frame_generation = Some(1);
    assert!(matches!(
        context.webdriver_cookies(id, &snapshot, true),
        Err(RuntimeError::ProtocolViolation(_))
    ));
    assert!(matches!(
        context.webdriver_permission_origin(id, &snapshot),
        Err(RuntimeError::ProtocolViolation(_))
    ));
    assert!(matches!(
        context.webdriver_upload_files_in_context(
            id,
            &snapshot,
            &[std::path::PathBuf::from(
                "/not-read-in-invalid-selected-frame"
            )]
        ),
        Err(RuntimeError::ProtocolViolation(_))
    ));
    assert_eq!(store.webdriver_cookies(&first, false).unwrap().len(), 1);
    snapshot.parent_realm = None;
    snapshot.frame_generation = None;
    WEBDRIVER_COOKIE_ADMISSION_FAULT.with(|fault| fault.set(2));
    context.webdriver_cookies(id, &snapshot, true).unwrap();
    assert_eq!(store.webdriver_cookies(&first, false).unwrap().len(), 1);
    assert!(store.webdriver_cookies(&second, false).unwrap().is_empty());
    context.commit_page_url(id, first.to_string()).unwrap();
    WEBDRIVER_COOKIE_ADMISSION_FAULT.with(|fault| fault.set(1));
    assert!(context.webdriver_cookies(id, &snapshot, true).is_err());
    assert_eq!(store.webdriver_cookies(&first, false).unwrap().len(), 1);
    page.close().await.unwrap();
}

#[tokio::test]
async fn permission_grants_cannot_resurrect_after_context_retirement() {
    use oryn_js::platform_control::{PermissionDescriptor, PermissionState};
    let context = Browser::in_process_probe().new_context_with_options(ContextOptions::default());
    let descriptor = PermissionDescriptor {
        name: "geolocation".into(),
        sysex: None,
        pan_tilt_zoom: None,
        allow_without_gesture: None,
    };
    context
        .webdriver_set_permission(
            "https://permission.test",
            descriptor.clone(),
            PermissionState::Denied,
        )
        .unwrap();
    context.retire_workers().await.unwrap();
    assert!(matches!(
        context.webdriver_set_permission(
            "https://permission.test",
            descriptor.clone(),
            PermissionState::Granted
        ),
        Err(RuntimeError::Closed)
    ));
    assert!(matches!(
        context
            .inner
            .platform_control
            .lock()
            .unwrap()
            .query(
                PageId(0),
                "https://permission.test",
                oryn_js::platform_control::Request::Permission { descriptor }
            )
            .unwrap(),
        oryn_js::platform_control::Response::Permission {
            state: PermissionState::Denied,
            ..
        }
    ));
}
