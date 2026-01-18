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

#[tokio::test]
async fn test_verify_pattern_gone() {
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

    let cond = Condition::PatternGone("search".to_string());
    assert!(verifier.verify(&cond, &ctx).await.unwrap());

    let cond_fail = Condition::PatternGone("login".to_string());
    assert!(!verifier.verify(&cond_fail, &ctx).await.unwrap());
}

#[tokio::test]
async fn test_verify_pattern_gone_no_patterns() {
    let scan = make_scan_result(vec![], None);
    let ctx = VerifierContext::new(&scan);
    let verifier = Verifier;

    let cond = Condition::PatternGone("login".to_string());
    assert!(verifier.verify(&cond, &ctx).await.unwrap());
}

#[tokio::test]
async fn test_verify_url_matches_regex() {
    let scan = make_scan_result(vec![], None);
    let ctx = VerifierContext::new(&scan);
    let verifier = Verifier;

    // Test valid regex
    let cond = Condition::UrlMatches(r"^https://.*\.com/login$".to_string());
    assert!(verifier.verify(&cond, &ctx).await.unwrap());

    let cond_fail = Condition::UrlMatches(r"^http://".to_string());
    assert!(!verifier.verify(&cond_fail, &ctx).await.unwrap());

    // Test invalid regex fallback (contains)
    let _cond_fallback = Condition::UrlMatches("example.com".to_string());
    // Force invalid regex syntax that is valid string
    // "log[in" is invalid regex because unclosed bracket.
    // But it might be in the string?
    // "https://example.com/login" does NOT contain "log[in".
    // So verify should return false.
    let cond_invalid_regex = Condition::UrlMatches("log[in".to_string());
    assert!(!verifier.verify(&cond_invalid_regex, &ctx).await.unwrap());

    // Test simple contains fallback
    assert!(
        verifier
            .verify(&Condition::UrlMatches("login".to_string()), &ctx)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_verify_hidden() {
    let el = make_element(1, Some("Submit"), Some("button"));
    let scan = make_scan_result(vec![el], None);
    let ctx = VerifierContext::new(&scan);
    let verifier = Verifier;

    // Visible el
    let target_visible = TargetSpec {
        kind: TargetKind::Id { id: 1 },
        fallback: None,
    };

    // Check if HIDDEN(id=1) -> False
    assert!(
        !verifier
            .verify(&Condition::Hidden(target_visible.clone()), &ctx)
            .await
            .unwrap()
    );

    // Non-existent element
    let target_hidden = TargetSpec {
        kind: TargetKind::Id { id: 99 },
        fallback: None,
    };
    // Check if HIDDEN(id=99) -> True
    assert!(
        verifier
            .verify(&Condition::Hidden(target_hidden), &ctx)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_verify_text_contains() {
    let el1 = make_element(1, Some("Hello World"), None);
    let el2 = make_element(2, Some("Goodbye"), Some("footer"));
    let scan = make_scan_result(vec![el1, el2], None);
    let ctx = VerifierContext::new(&scan);
    let verifier = Verifier;

    // Global search
    assert!(
        verifier
            .verify(
                &Condition::TextContains {
                    text: "Hello".to_string(),
                    within: None
                },
                &ctx
            )
            .await
            .unwrap()
    );

    assert!(
        verifier
            .verify(
                &Condition::TextContains {
                    text: "World".to_string(),
                    within: None
                },
                &ctx
            )
            .await
            .unwrap()
    );

    assert!(
        !verifier
            .verify(
                &Condition::TextContains {
                    text: "NotFound".to_string(),
                    within: None
                },
                &ctx
            )
            .await
            .unwrap()
    );

    // Scoped search
    let within_target = Some(TargetSpec {
        kind: TargetKind::Id { id: 1 },
        fallback: None,
    });

    assert!(
        verifier
            .verify(
                &Condition::TextContains {
                    text: "Hello".to_string(),
                    within: within_target.clone()
                },
                &ctx
            )
            .await
            .unwrap()
    );

    // "Goodbye" is NOT in id=1
    assert!(
        !verifier
            .verify(
                &Condition::TextContains {
                    text: "Goodbye".to_string(),
                    within: within_target
                },
                &ctx
            )
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_verify_count() {
    let el1 = make_element(1, None, None); // div
    let el2 = make_element(2, None, None); // div
    let el3 = make_element(3, None, None); // div
    let scan = make_scan_result(vec![el1, el2, el3], None);
    // Element selector is format!("#el-{}", id), element_type is "div"
    // Condition::Count checks selector match OR type match

    let ctx = VerifierContext::new(&scan);
    let verifier = Verifier;

    // Count divs = 3
    assert!(
        verifier
            .verify(
                &Condition::Count {
                    selector: "div".to_string(),
                    min: Some(3),
                    max: Some(3)
                },
                &ctx
            )
            .await
            .unwrap()
    );

    // Min 2 -> pass
    assert!(
        verifier
            .verify(
                &Condition::Count {
                    selector: "div".to_string(),
                    min: Some(2),
                    max: None
                },
                &ctx
            )
            .await
            .unwrap()
    );

    // Max 2 -> fail
    assert!(
        !verifier
            .verify(
                &Condition::Count {
                    selector: "div".to_string(),
                    min: None,
                    max: Some(2)
                },
                &ctx
            )
            .await
            .unwrap()
    );

    // Count exact 3 -> pass
    assert!(
        verifier
            .verify(
                &Condition::Count {
                    selector: "div".to_string(),
                    min: Some(3),
                    max: Some(3)
                },
                &ctx
            )
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_verify_expression_true_boolean() {
    let scan = make_scan_result(vec![], None);
    let mut variables = HashMap::new();
    variables.insert("flag".to_string(), serde_json::json!(true));

    let ctx = VerifierContext::with_variables(&scan, &variables);
    let verifier = Verifier;

    let cond = Condition::Expression("$flag".to_string());
    assert!(verifier.verify(&cond, &ctx).await.unwrap());
}

#[tokio::test]
async fn test_verify_expression_false_boolean() {
    let scan = make_scan_result(vec![], None);
    let mut variables = HashMap::new();
    variables.insert("disabled".to_string(), serde_json::json!(false));

    let ctx = VerifierContext::with_variables(&scan, &variables);
    let verifier = Verifier;

    let cond = Condition::Expression("$disabled".to_string());
    assert!(!verifier.verify(&cond, &ctx).await.unwrap());
}

#[tokio::test]
async fn test_verify_expression_truthy_string() {
    let scan = make_scan_result(vec![], None);
    let mut variables = HashMap::new();
    variables.insert("name".to_string(), serde_json::json!("hello"));
    variables.insert("empty".to_string(), serde_json::json!(""));
    variables.insert("false_str".to_string(), serde_json::json!("false"));
    variables.insert("zero_str".to_string(), serde_json::json!("0"));

    let ctx = VerifierContext::with_variables(&scan, &variables);
    let verifier = Verifier;

    // Non-empty string is truthy
    assert!(
        verifier
            .verify(&Condition::Expression("$name".to_string()), &ctx)
            .await
            .unwrap()
    );

    // Empty string is falsy
    assert!(
        !verifier
            .verify(&Condition::Expression("$empty".to_string()), &ctx)
            .await
            .unwrap()
    );

    // "false" string is falsy
    assert!(
        !verifier
            .verify(&Condition::Expression("$false_str".to_string()), &ctx)
            .await
            .unwrap()
    );

    // "0" string is falsy
    assert!(
        !verifier
            .verify(&Condition::Expression("$zero_str".to_string()), &ctx)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_verify_expression_missing_variable() {
    let scan = make_scan_result(vec![], None);
    let variables = HashMap::new();

    let ctx = VerifierContext::with_variables(&scan, &variables);
    let verifier = Verifier;

    // Missing variable is falsy
    let cond = Condition::Expression("$nonexistent".to_string());
    assert!(!verifier.verify(&cond, &ctx).await.unwrap());
}

#[tokio::test]
async fn test_verify_expression_literal() {
    let scan = make_scan_result(vec![], None);
    let ctx = VerifierContext::new(&scan);
    let verifier = Verifier;

    // Literal "true" is truthy
    assert!(
        verifier
            .verify(&Condition::Expression("true".to_string()), &ctx)
            .await
            .unwrap()
    );

    // Literal "1" is truthy
    assert!(
        verifier
            .verify(&Condition::Expression("1".to_string()), &ctx)
            .await
            .unwrap()
    );

    // Literal "false" is falsy
    assert!(
        !verifier
            .verify(&Condition::Expression("false".to_string()), &ctx)
            .await
            .unwrap()
    );

    // Literal "0" is falsy
    assert!(
        !verifier
            .verify(&Condition::Expression("0".to_string()), &ctx)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_verify_expression_number_truthy() {
    let scan = make_scan_result(vec![], None);
    let mut variables = HashMap::new();
    variables.insert("positive".to_string(), serde_json::json!(42));
    variables.insert("zero".to_string(), serde_json::json!(0));
    variables.insert("negative".to_string(), serde_json::json!(-1));

    let ctx = VerifierContext::with_variables(&scan, &variables);
    let verifier = Verifier;

    // Positive number is truthy
    assert!(
        verifier
            .verify(&Condition::Expression("$positive".to_string()), &ctx)
            .await
            .unwrap()
    );

    // Zero is falsy
    assert!(
        !verifier
            .verify(&Condition::Expression("$zero".to_string()), &ctx)
            .await
            .unwrap()
    );

    // Negative number is truthy
    assert!(
        verifier
            .verify(&Condition::Expression("$negative".to_string()), &ctx)
            .await
            .unwrap()
    );
}
