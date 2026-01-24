use oryn_engine::command::{Command, Target};
use oryn_engine::protocol::{
    DetectedPatterns, Element, ElementState, LoginPattern, PageInfo, Rect, ScanResult, ScanStats,
    ScrollInfo, ViewportInfo,
};
use oryn_engine::resolution::engine::ResolutionEngine;
use std::collections::HashMap;

/// Mock backend for testing
#[derive(Debug, Default)]
struct MockBackend;

#[async_trait::async_trait]
impl oryn_engine::backend::Backend for MockBackend {
    async fn launch(&mut self) -> Result<(), oryn_engine::backend::BackendError> {
        Ok(())
    }
    async fn close(&mut self) -> Result<(), oryn_engine::backend::BackendError> {
        Ok(())
    }
    async fn is_ready(&self) -> bool {
        true
    }
    async fn navigate(
        &mut self,
        _url: &str,
    ) -> Result<oryn_engine::backend::NavigationResult, oryn_engine::backend::BackendError> {
        unimplemented!()
    }
    async fn go_back(
        &mut self,
    ) -> Result<oryn_engine::backend::NavigationResult, oryn_engine::backend::BackendError> {
        unimplemented!()
    }
    async fn go_forward(
        &mut self,
    ) -> Result<oryn_engine::backend::NavigationResult, oryn_engine::backend::BackendError> {
        unimplemented!()
    }
    async fn refresh(
        &mut self,
    ) -> Result<oryn_engine::backend::NavigationResult, oryn_engine::backend::BackendError> {
        unimplemented!()
    }
    async fn screenshot(&mut self) -> Result<Vec<u8>, oryn_engine::backend::BackendError> {
        unimplemented!()
    }
    async fn pdf(&mut self) -> Result<Vec<u8>, oryn_engine::backend::BackendError> {
        unimplemented!()
    }
    async fn get_cookies(
        &mut self,
    ) -> Result<Vec<oryn_engine::protocol::Cookie>, oryn_engine::backend::BackendError> {
        unimplemented!()
    }
    async fn set_cookie(
        &mut self,
        _cookie: oryn_engine::protocol::Cookie,
    ) -> Result<(), oryn_engine::backend::BackendError> {
        unimplemented!()
    }
    async fn get_tabs(
        &mut self,
    ) -> Result<Vec<oryn_engine::protocol::TabInfo>, oryn_engine::backend::BackendError> {
        unimplemented!()
    }
    async fn press_key(
        &mut self,
        _key: &str,
        _modifiers: &[String],
    ) -> Result<(), oryn_engine::backend::BackendError> {
        unimplemented!()
    }
    async fn execute_scanner(
        &mut self,
        _command: oryn_engine::protocol::ScannerRequest,
    ) -> Result<oryn_engine::protocol::ScannerProtocolResponse, oryn_engine::backend::BackendError>
    {
        unimplemented!()
    }
}

fn make_element(id: u32, type_: &str, text: Option<&str>, role: Option<&str>) -> Element {
    let mut attributes = HashMap::new();
    if type_ == "input" {
        attributes.insert("type".to_string(), "text".to_string());
    }
    if let Some(r) = role {
        attributes.insert("role".to_string(), r.to_string());
    }

    Element {
        id,
        element_type: type_.to_string(),
        role: role.map(|s| s.to_string()),
        text: text.map(|s| s.to_string()),
        label: None,
        value: None,
        placeholder: None,
        selector: format!("#{}", id),
        xpath: None,
        rect: Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 30.0,
        },
        attributes,
        state: ElementState::default(),
        children: vec![],
    }
}

#[tokio::test]
async fn test_inference_single_form() {
    let mut backend = MockBackend;

    let form = make_element(1, "form", None, None);
    let mut submit_btn = make_element(2, "button", Some("Submit"), None);
    submit_btn
        .attributes
        .insert("type".to_string(), "submit".to_string());
    // Put button inside form
    let mut form = form;
    form.rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 500.0,
        height: 500.0,
    };
    submit_btn.rect = Rect {
        x: 50.0,
        y: 50.0,
        width: 100.0,
        height: 30.0,
    };

    let scan = ScanResult {
        page: PageInfo {
            url: "http://test.com".into(),
            title: "Test".into(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements: vec![form, submit_btn],
        stats: ScanStats {
            total: 2,
            scanned: 2,
        },
        patterns: None,
        changes: None,
        available_intents: None,
    };

    // "submit" without target -> inferred
    let cmd = Command::Submit(Target::Infer);
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok());
    match result.unwrap() {
        Command::Submit(Target::Id(id)) => assert_eq!(id, 2),
        _ => panic!("Expected resolved ID"),
    }
}

#[tokio::test]
async fn test_inference_login_pattern() {
    let mut backend = MockBackend;

    let email = make_element(1, "input", None, None);
    let password = make_element(2, "input", None, None);
    let submit = make_element(3, "button", Some("Log In"), None);

    let patterns = DetectedPatterns {
        login: Some(LoginPattern {
            email: Some(1),
            username: None,
            password: 2,
            submit: Some(3),
            remember: None,
        }),
        ..Default::default()
    };

    let scan = ScanResult {
        page: PageInfo {
            url: "http://test.com".into(),
            title: "Test".into(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements: vec![email, password, submit],
        stats: ScanStats {
            total: 3,
            scanned: 3,
        },
        patterns: Some(patterns),
        changes: None,
        available_intents: None,
    };

    let cmd = Command::Submit(Target::Infer);
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok());
    match result.unwrap() {
        Command::Submit(Target::Id(id)) => assert_eq!(id, 3), // From pattern
        _ => panic!("Expected resolved ID"),
    }
}

#[tokio::test]
async fn test_inference_any_submit() {
    let mut backend = MockBackend;

    // Just a submit button, no form container (maybe body is form-like or just loose)
    let mut submit_btn = make_element(5, "button", Some("Save"), None);
    submit_btn
        .attributes
        .insert("type".to_string(), "submit".to_string());

    let scan = ScanResult {
        page: PageInfo {
            url: "http://test.com".into(),
            title: "Test".into(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements: vec![submit_btn],
        stats: ScanStats {
            total: 1,
            scanned: 1,
        },
        patterns: None,
        changes: None,
        available_intents: None,
    };

    let cmd = Command::Submit(Target::Infer);
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok());
    match result.unwrap() {
        Command::Submit(Target::Id(id)) => assert_eq!(id, 5),
        _ => panic!("Expected resolved ID"),
    }
}

#[tokio::test]
async fn test_inference_fail_no_candidate() {
    let mut backend = MockBackend;

    // No buttons, no forms
    let div = make_element(1, "div", Some("Hello"), None);

    let scan = ScanResult {
        page: PageInfo {
            url: "http://test.com".into(),
            title: "Test".into(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements: vec![div],
        stats: ScanStats {
            total: 1,
            scanned: 1,
        },
        patterns: None,
        changes: None,
        available_intents: None,
    };

    let cmd = Command::Submit(Target::Infer);
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.target, "Infer");
    assert!(err.reason.contains("No inference rule satisfied"));
}

#[tokio::test]
async fn test_inference_dismiss_modal() {
    let mut backend = MockBackend;

    let modal = make_element(1, "dialog", None, None);
    let mut close_btn = make_element(2, "button", Some("Close"), None);
    close_btn
        .attributes
        .insert("aria-label".to_string(), "Close".to_string());

    // Put button inside modal (conceptually via scoped context logic, but MockBackend doesn't do spatial check really well
    // unless I set up rects properly.
    // In inference.rs `any_modal_close` uses `scoped_to` which uses `is_inside`.
    // So I must set rects.
    let mut modal = modal;
    modal.rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 400.0,
        height: 300.0,
    };
    close_btn.rect = Rect {
        x: 350.0,
        y: 10.0,
        width: 30.0,
        height: 30.0,
    };

    let scan = ScanResult {
        page: PageInfo {
            url: "http://test.com".into(),
            title: "Test".into(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements: vec![modal, close_btn],
        stats: ScanStats {
            total: 2,
            scanned: 2,
        },
        patterns: None,
        changes: None,
        available_intents: None,
    };

    // "dismiss" -> inferred
    let cmd = Command::Dismiss(Target::Infer, HashMap::new());
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok());
    // Should resolve to Click(close_btn)
    match result.unwrap() {
        Command::Click(Target::Id(id), _) => assert_eq!(id, 2),
        _ => panic!("Expected Click command"),
    }
}

#[tokio::test]
async fn test_inference_accept_cookies() {
    let mut backend = MockBackend;

    // Cookie banner pattern
    let banner = make_element(1, "div", None, None);
    let accept_btn = make_element(2, "button", Some("Accept All"), None);

    let patterns = DetectedPatterns {
        cookie_banner: Some(oryn_engine::protocol::CookieBannerPattern {
            accept: Some(2),
            reject: None,
            settings: None,
        }),
        ..Default::default()
    };

    let scan = ScanResult {
        page: PageInfo {
            url: "http://test.com".into(),
            title: "Test".into(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements: vec![banner, accept_btn],
        stats: ScanStats {
            total: 2,
            scanned: 2,
        },
        patterns: Some(patterns),
        changes: None,
        available_intents: None,
    };

    // "accept" -> inferred
    let cmd = Command::Accept(Target::Infer, HashMap::new());
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok());
    match result.unwrap() {
        Command::Click(Target::Id(id), _) => assert_eq!(id, 2),
        _ => panic!("Expected Click command"),
    }
}
