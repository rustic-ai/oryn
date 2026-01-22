use oryn_core::command::{
    Command, ExtractSource, StorageAction, StorageType, Target, WaitCondition,
};
use oryn_core::protocol::{MouseButton, ScannerRequest, ScrollDirection};
use oryn_core::translator::translate;
use std::collections::HashMap;

/// Creates a HashMap of options from key-value pairs.
fn opts(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn test_translate_click() {
    let cmd = Command::Click(Target::Id(5), HashMap::new());
    let ScannerRequest::Click(c) = translate(&cmd).unwrap() else {
        panic!("Expected Click request");
    };
    assert_eq!(c.id, 5);
    assert!(!c.force);
    assert!(!c.double);
    assert!(matches!(c.button, MouseButton::Left));
}

#[test]
fn test_translate_click_with_options() {
    // Test --force option
    let cmd = Command::Click(Target::Id(5), opts(&[("force", "true")]));
    let ScannerRequest::Click(c) = translate(&cmd).unwrap() else {
        panic!("Expected Click request");
    };
    assert!(c.force);

    // Test --double option
    let cmd = Command::Click(Target::Id(5), opts(&[("double", "true")]));
    let ScannerRequest::Click(c) = translate(&cmd).unwrap() else {
        panic!("Expected Click request");
    };
    assert!(c.double);

    // Test --right option
    let cmd = Command::Click(Target::Id(5), opts(&[("right", "true")]));
    let ScannerRequest::Click(c) = translate(&cmd).unwrap() else {
        panic!("Expected Click request");
    };
    assert!(matches!(c.button, MouseButton::Right));

    // Test --middle option
    let cmd = Command::Click(Target::Id(5), opts(&[("middle", "true")]));
    let ScannerRequest::Click(c) = translate(&cmd).unwrap() else {
        panic!("Expected Click request");
    };
    assert!(matches!(c.button, MouseButton::Middle));

    // Test combined options
    let cmd = Command::Click(
        Target::Id(5),
        opts(&[("force", "true"), ("double", "true")]),
    );
    let ScannerRequest::Click(c) = translate(&cmd).unwrap() else {
        panic!("Expected Click request");
    };
    assert!(c.force);
    assert!(c.double);
}

#[test]
fn test_translate_type() {
    let cmd = Command::Type(Target::Id(10), "hello".into(), HashMap::new());
    let ScannerRequest::Type(t) = translate(&cmd).unwrap() else {
        panic!("Expected Type request");
    };
    assert_eq!(t.id, 10);
    assert_eq!(t.text, "hello");
    assert!(t.clear); // Default is to clear
    assert!(!t.submit);
    assert!(t.delay.is_none());
}

#[test]
fn test_translate_type_with_options() {
    // Test --append option (inverse of clear)
    let cmd = Command::Type(Target::Id(10), "hello".into(), opts(&[("append", "true")]));
    let ScannerRequest::Type(t) = translate(&cmd).unwrap() else {
        panic!("Expected Type request");
    };
    assert!(!t.clear);

    // Test --enter option (submit)
    let cmd = Command::Type(Target::Id(10), "hello".into(), opts(&[("enter", "true")]));
    let ScannerRequest::Type(t) = translate(&cmd).unwrap() else {
        panic!("Expected Type request");
    };
    assert!(t.submit);

    // Test --delay option
    let cmd = Command::Type(Target::Id(10), "hello".into(), opts(&[("delay", "50")]));
    let ScannerRequest::Type(t) = translate(&cmd).unwrap() else {
        panic!("Expected Type request");
    };
    assert_eq!(t.delay, Some(50));

    // Test combined options
    let cmd = Command::Type(
        Target::Id(10),
        "hello".into(),
        opts(&[("append", "true"), ("enter", "true"), ("delay", "100")]),
    );
    let ScannerRequest::Type(t) = translate(&cmd).unwrap() else {
        panic!("Expected Type request");
    };
    assert!(!t.clear);
    assert!(t.submit);
    assert_eq!(t.delay, Some(100));
}

#[test]
fn test_translate_scroll() {
    let cmd = Command::Scroll(None, opts(&[("direction", "up")]));
    let ScannerRequest::Scroll(s) = translate(&cmd).unwrap() else {
        panic!("Expected Scroll request");
    };
    assert!(s.id.is_none());
    assert!(matches!(s.direction, ScrollDirection::Up));
}

#[test]
fn test_translate_wait_visible() {
    let cmd = Command::Wait(WaitCondition::Visible(Target::Id(123)), HashMap::new());
    let ScannerRequest::Wait(w) = translate(&cmd).unwrap() else {
        panic!("Expected Wait request");
    };
    assert_eq!(w.condition, "visible");
    assert_eq!(w.target.unwrap(), "123");
}

#[test]
fn test_translate_storage() {
    // Test clear both storages
    let cmd = Command::Storage(StorageAction::Clear {
        storage_type: StorageType::Both,
    });
    let ScannerRequest::Execute(e) = translate(&cmd).unwrap() else {
        panic!("Expected Execute request for storage clear");
    };
    assert!(e.script.contains("localStorage.clear()"));
    assert!(e.script.contains("sessionStorage.clear()"));

    // Test get from local storage
    let cmd = Command::Storage(StorageAction::Get {
        storage_type: StorageType::Local,
        key: "myKey".into(),
    });
    let ScannerRequest::Execute(e) = translate(&cmd).unwrap() else {
        panic!("Expected Execute request for storage get");
    };
    assert!(e.script.contains("localStorage.getItem"));
    assert!(e.script.contains("myKey"));

    // Test set in session storage
    let cmd = Command::Storage(StorageAction::Set {
        storage_type: StorageType::Session,
        key: "theme".into(),
        value: "dark".into(),
    });
    let ScannerRequest::Execute(e) = translate(&cmd).unwrap() else {
        panic!("Expected Execute request for storage set");
    };
    assert!(e.script.contains("sessionStorage.setItem"));
    assert!(e.script.contains("theme"));
    assert!(e.script.contains("dark"));

    // Test list local storage
    let cmd = Command::Storage(StorageAction::List {
        storage_type: StorageType::Local,
    });
    let ScannerRequest::Execute(e) = translate(&cmd).unwrap() else {
        panic!("Expected Execute request for storage list");
    };
    assert!(e.script.contains("Object.keys(localStorage)"));
}

#[test]
fn test_translate_extract() {
    let cmd = Command::Extract(ExtractSource::Links);
    let ScannerRequest::Extract(e) = translate(&cmd).unwrap() else {
        panic!("Expected Extract request");
    };
    assert_eq!(e.source, "links");
}

#[test]
fn test_translate_login() {
    let cmd = Command::Login("user".into(), "pass".into(), HashMap::new());
    let ScannerRequest::Login(l) = translate(&cmd).unwrap() else {
        panic!("Expected Login request");
    };
    assert_eq!(l.username, "user");
    assert_eq!(l.password, "pass");
}

#[test]
fn test_translate_search() {
    let cmd = Command::Search("oryn".into(), HashMap::new());
    let ScannerRequest::Search(s) = translate(&cmd).unwrap() else {
        panic!("Expected Search request");
    };
    assert_eq!(s.query, "oryn");
}

#[test]
fn test_translate_accept() {
    let cmd = Command::Accept(Target::Text("cookies".into()), HashMap::new());
    let ScannerRequest::Accept(a) = translate(&cmd).unwrap() else {
        panic!("Expected Accept request");
    };
    assert_eq!(a.target, "cookies");
}

#[test]
fn test_translate_observe() {
    let cmd = Command::Observe(HashMap::new());
    let ScannerRequest::Scan(s) = translate(&cmd).unwrap() else {
        panic!("Expected Scan request");
    };
    assert!(s.max_elements.is_none());
    assert!(s.near.is_none());
    assert!(!s.viewport_only);
    assert!(!s.view_all);
    assert!(!s.include_hidden);
}

#[test]
fn test_translate_observe_with_options() {
    // Test --near option
    let cmd = Command::Observe(opts(&[("near", "Login")]));
    let ScannerRequest::Scan(s) = translate(&cmd).unwrap() else {
        panic!("Expected Scan request");
    };
    assert_eq!(s.near, Some("Login".into()));

    // Test --viewport option
    let cmd = Command::Observe(opts(&[("viewport", "true")]));
    let ScannerRequest::Scan(s) = translate(&cmd).unwrap() else {
        panic!("Expected Scan request");
    };
    assert!(s.viewport_only);

    // Test --max option
    let cmd = Command::Observe(opts(&[("max", "50")]));
    let ScannerRequest::Scan(s) = translate(&cmd).unwrap() else {
        panic!("Expected Scan request");
    };
    assert_eq!(s.max_elements, Some(50));

    // Test --full option
    let cmd = Command::Observe(opts(&[("full", "true")]));
    let ScannerRequest::Scan(s) = translate(&cmd).unwrap() else {
        panic!("Expected Scan request");
    };
    assert!(s.view_all);

    // Test --hidden option
    let cmd = Command::Observe(opts(&[("hidden", "true")]));
    let ScannerRequest::Scan(s) = translate(&cmd).unwrap() else {
        panic!("Expected Scan request");
    };
    assert!(s.include_hidden);

    // Test combined options
    let cmd = Command::Observe(opts(&[
        ("near", "Submit"),
        ("viewport", "true"),
        ("max", "100"),
    ]));
    let ScannerRequest::Scan(s) = translate(&cmd).unwrap() else {
        panic!("Expected Scan request");
    };
    assert_eq!(s.near, Some("Submit".into()));
    assert!(s.viewport_only);
    assert_eq!(s.max_elements, Some(100));
}
