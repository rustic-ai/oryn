use oryn_core::intent::definition::{Condition, TargetKind, TargetSpec};
use oryn_core::intent::verifier::{Verifier, VerifierContext};
use oryn_core::protocol::{
    DetectedPatterns, Element, ElementState, PageInfo, Rect, ScanResult, ScanStats, ScrollInfo,
    ViewportInfo,
};
use std::collections::HashMap;

fn make_scan_result(elements: Vec<Element>, patterns: Option<DetectedPatterns>) -> ScanResult {
    ScanResult {
        page: PageInfo {
            url: "https://example.com/login".to_string(),
            title: "Login Page".to_string(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements,
        stats: ScanStats {
            total: 0,
            scanned: 0,
        },
        patterns,
        changes: None,
    }
}

fn make_element(id: u32, text: Option<&str>, role: Option<&str>) -> Element {
    Element {
        id,
        element_type: "div".to_string(),
        role: role.map(|s| s.to_string()),
        text: text.map(|s| s.to_string()),
        label: None,
        value: None,
        placeholder: None,
        selector: format!("#el-{}", id),
        xpath: None,
        rect: Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        },
        attributes: HashMap::new(),
        state: ElementState::default(),
        children: vec![],
    }
}

#[tokio::test]
async fn test_verify_pattern_exists() {
    let patterns = DetectedPatterns {
        login: Some(oryn_core::protocol::LoginPattern {
            email: None,
            username: None,
            password: 0,
            submit: None,
            remember: None,
        }),
        ..Default::default()
    };

    let scan = make_scan_result(vec![], Some(patterns));
    let ctx = VerifierContext::new(&scan);
    let verifier = Verifier;

    let cond = Condition::PatternExists("login".to_string());
    assert!(verifier.verify(&cond, &ctx).await.unwrap());

    let cond_fail = Condition::PatternExists("search".to_string());
    assert!(!verifier.verify(&cond_fail, &ctx).await.unwrap());
}

#[tokio::test]
async fn test_verify_url_contains() {
    let scan = make_scan_result(vec![], None);
    let ctx = VerifierContext::new(&scan);
    let verifier = Verifier;

    let cond = Condition::UrlContains(vec!["example.com".to_string()]);
    assert!(verifier.verify(&cond, &ctx).await.unwrap());

    let cond_fail = Condition::UrlContains(vec!["google.com".to_string()]);
    assert!(!verifier.verify(&cond_fail, &ctx).await.unwrap());
}

#[tokio::test]
async fn test_verify_visible() {
    let el = make_element(1, Some("Submit"), Some("button"));
    let scan = make_scan_result(vec![el], None);
    let ctx = VerifierContext::new(&scan);
    let verifier = Verifier;

    let target = TargetSpec {
        kind: TargetKind::Text {
            text: "Submit".to_string(),
            match_type: Default::default(),
        },
        fallback: None,
    };

    let cond = Condition::Visible(target);
    assert!(verifier.verify(&cond, &ctx).await.unwrap());

    let target_fail = TargetSpec {
        kind: TargetKind::Text {
            text: "Cancel".to_string(),
            match_type: Default::default(),
        },
        fallback: None,
    };
    let cond_fail = Condition::Visible(target_fail);
    assert!(!verifier.verify(&cond_fail, &ctx).await.unwrap());
}

#[tokio::test]
async fn test_verify_logic_operators() {
    let scan = make_scan_result(vec![], None);
    let ctx = VerifierContext::new(&scan);
    let verifier = Verifier;

    let true_cond = Condition::UrlContains(vec!["example".to_string()]);
    let false_cond = Condition::UrlContains(vec!["google".to_string()]);

    // All
    assert!(
        verifier
            .verify(&Condition::All(vec![true_cond.clone()]), &ctx)
            .await
            .unwrap()
    );
    assert!(
        !verifier
            .verify(
                &Condition::All(vec![true_cond.clone(), false_cond.clone()]),
                &ctx
            )
            .await
            .unwrap()
    );

    // Any
    assert!(
        verifier
            .verify(
                &Condition::Any(vec![false_cond.clone(), true_cond.clone()]),
                &ctx
            )
            .await
            .unwrap()
    );
    assert!(
        !verifier
            .verify(&Condition::Any(vec![false_cond.clone()]), &ctx)
            .await
            .unwrap()
    );

    // Any Empty (should be false)
    assert!(
        !verifier
            .verify(&Condition::Any(vec![]), &ctx)
            .await
            .unwrap()
    );
}
