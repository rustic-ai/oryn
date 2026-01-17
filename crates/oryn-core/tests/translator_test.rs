use oryn_core::command::{Command, ExtractSource, StorageAction, StorageType, Target, WaitCondition};
use oryn_core::protocol::ScannerRequest;
use oryn_core::translator::translate;
use std::collections::HashMap;

#[test]
fn test_translate_click() {
    let cmd = Command::Click(Target::Id(5), HashMap::new());
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Click(c) = req {
        assert_eq!(c.id, 5);
        assert!(!c.force);
        assert!(!c.double);
        assert!(matches!(c.button, oryn_core::protocol::MouseButton::Left));
    } else {
        panic!("Wrong request type");
    }
}

#[test]
fn test_translate_click_with_options() {
    // Test --force option
    let mut opts = HashMap::new();
    opts.insert("force".to_string(), "true".to_string());
    let cmd = Command::Click(Target::Id(5), opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Click(c) = req {
        assert!(c.force);
    } else {
        panic!("Wrong request type");
    }

    // Test --double option
    let mut opts = HashMap::new();
    opts.insert("double".to_string(), "true".to_string());
    let cmd = Command::Click(Target::Id(5), opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Click(c) = req {
        assert!(c.double);
    } else {
        panic!("Wrong request type");
    }

    // Test --right option
    let mut opts = HashMap::new();
    opts.insert("right".to_string(), "true".to_string());
    let cmd = Command::Click(Target::Id(5), opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Click(c) = req {
        assert!(matches!(c.button, oryn_core::protocol::MouseButton::Right));
    } else {
        panic!("Wrong request type");
    }

    // Test --middle option
    let mut opts = HashMap::new();
    opts.insert("middle".to_string(), "true".to_string());
    let cmd = Command::Click(Target::Id(5), opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Click(c) = req {
        assert!(matches!(c.button, oryn_core::protocol::MouseButton::Middle));
    } else {
        panic!("Wrong request type");
    }

    // Test combined options
    let mut opts = HashMap::new();
    opts.insert("force".to_string(), "true".to_string());
    opts.insert("double".to_string(), "true".to_string());
    let cmd = Command::Click(Target::Id(5), opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Click(c) = req {
        assert!(c.force);
        assert!(c.double);
    } else {
        panic!("Wrong request type");
    }
}

#[test]
fn test_translate_type() {
    let cmd = Command::Type(Target::Id(10), "hello".to_string(), HashMap::new());
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Type(t) = req {
        assert_eq!(t.id, 10);
        assert_eq!(t.text, "hello");
        assert!(t.clear); // Default is to clear
        assert!(!t.submit);
        assert!(t.delay.is_none());
    } else {
        panic!("Wrong request type");
    }
}

#[test]
fn test_translate_type_with_options() {
    // Test --append option (inverse of clear)
    let mut opts = HashMap::new();
    opts.insert("append".to_string(), "true".to_string());
    let cmd = Command::Type(Target::Id(10), "hello".to_string(), opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Type(t) = req {
        assert!(!t.clear); // append means don't clear
    } else {
        panic!("Wrong request type");
    }

    // Test --enter option (submit)
    let mut opts = HashMap::new();
    opts.insert("enter".to_string(), "true".to_string());
    let cmd = Command::Type(Target::Id(10), "hello".to_string(), opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Type(t) = req {
        assert!(t.submit);
    } else {
        panic!("Wrong request type");
    }

    // Test --delay option
    let mut opts = HashMap::new();
    opts.insert("delay".to_string(), "50".to_string());
    let cmd = Command::Type(Target::Id(10), "hello".to_string(), opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Type(t) = req {
        assert_eq!(t.delay, Some(50));
    } else {
        panic!("Wrong request type");
    }

    // Test combined options
    let mut opts = HashMap::new();
    opts.insert("append".to_string(), "true".to_string());
    opts.insert("enter".to_string(), "true".to_string());
    opts.insert("delay".to_string(), "100".to_string());
    let cmd = Command::Type(Target::Id(10), "hello".to_string(), opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Type(t) = req {
        assert!(!t.clear);
        assert!(t.submit);
        assert_eq!(t.delay, Some(100));
    } else {
        panic!("Wrong request type");
    }
}

#[test]
fn test_translate_scroll() {
    let mut opts = HashMap::new();
    opts.insert("direction".to_string(), "up".to_string());
    let cmd = Command::Scroll(None, opts);

    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Scroll(s) = req {
        assert!(s.id.is_none());
        assert!(matches!(
            s.direction,
            oryn_core::protocol::ScrollDirection::Up
        ));
    } else {
        panic!("Wrong request type");
    }
}

#[test]
fn test_translate_wait_visible() {
    let cmd = Command::Wait(WaitCondition::Visible(Target::Id(123)), HashMap::new());
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Wait(w) = req {
        assert_eq!(w.condition, "visible");
        assert_eq!(w.target.unwrap(), "123");
    } else {
        panic!("Wrong request type");
    }
}

#[test]
fn test_translate_storage() {
    // Test clear both storages
    let cmd = Command::Storage(StorageAction::Clear {
        storage_type: StorageType::Both,
    });
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Execute(e) = req {
        assert!(e.script.contains("localStorage.clear()"));
        assert!(e.script.contains("sessionStorage.clear()"));
    } else {
        panic!("Wrong request type for storage clear");
    }

    // Test get from local storage
    let cmd = Command::Storage(StorageAction::Get {
        storage_type: StorageType::Local,
        key: "myKey".to_string(),
    });
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Execute(e) = req {
        assert!(e.script.contains("localStorage.getItem"));
        assert!(e.script.contains("myKey"));
    } else {
        panic!("Wrong request type for storage get");
    }

    // Test set in session storage
    let cmd = Command::Storage(StorageAction::Set {
        storage_type: StorageType::Session,
        key: "theme".to_string(),
        value: "dark".to_string(),
    });
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Execute(e) = req {
        assert!(e.script.contains("sessionStorage.setItem"));
        assert!(e.script.contains("theme"));
        assert!(e.script.contains("dark"));
    } else {
        panic!("Wrong request type for storage set");
    }

    // Test list local storage
    let cmd = Command::Storage(StorageAction::List {
        storage_type: StorageType::Local,
    });
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Execute(e) = req {
        assert!(e.script.contains("Object.keys(localStorage)"));
    } else {
        panic!("Wrong request type for storage list");
    }
}

#[test]
fn test_translate_extract() {
    let cmd = Command::Extract(ExtractSource::Links);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Extract(e) = req {
        assert_eq!(e.source, "links");
    } else {
        panic!("Wrong request type");
    }
}

#[test]
fn test_translate_login() {
    let cmd = Command::Login("user".to_string(), "pass".to_string(), HashMap::new());
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Login(l) = req {
        assert_eq!(l.username, "user");
        assert_eq!(l.password, "pass");
    } else {
        panic!("Wrong request type");
    }
}

#[test]
fn test_translate_search() {
    let cmd = Command::Search("oryn".to_string(), HashMap::new());
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Search(s) = req {
        assert_eq!(s.query, "oryn");
    } else {
        panic!("Wrong request type");
    }
}

#[test]
fn test_translate_accept() {
    let cmd = Command::Accept("cookies".to_string(), HashMap::new());
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Accept(a) = req {
        assert_eq!(a.target, "cookies");
    } else {
        panic!("Wrong request type");
    }
}

#[test]
fn test_translate_observe() {
    // Test default observe (no options)
    let cmd = Command::Observe(HashMap::new());
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Scan(s) = req {
        assert!(s.max_elements.is_none());
        assert!(s.near.is_none());
        assert!(!s.viewport_only);
        assert!(!s.view_all);
        assert!(!s.include_hidden);
    } else {
        panic!("Wrong request type");
    }
}

#[test]
fn test_translate_observe_with_options() {
    // Test --near option
    let mut opts = HashMap::new();
    opts.insert("near".to_string(), "Login".to_string());
    let cmd = Command::Observe(opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Scan(s) = req {
        assert_eq!(s.near, Some("Login".to_string()));
    } else {
        panic!("Wrong request type");
    }

    // Test --viewport option
    let mut opts = HashMap::new();
    opts.insert("viewport".to_string(), "true".to_string());
    let cmd = Command::Observe(opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Scan(s) = req {
        assert!(s.viewport_only);
    } else {
        panic!("Wrong request type");
    }

    // Test --max option
    let mut opts = HashMap::new();
    opts.insert("max".to_string(), "50".to_string());
    let cmd = Command::Observe(opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Scan(s) = req {
        assert_eq!(s.max_elements, Some(50));
    } else {
        panic!("Wrong request type");
    }

    // Test --full option
    let mut opts = HashMap::new();
    opts.insert("full".to_string(), "true".to_string());
    let cmd = Command::Observe(opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Scan(s) = req {
        assert!(s.view_all);
    } else {
        panic!("Wrong request type");
    }

    // Test --hidden option
    let mut opts = HashMap::new();
    opts.insert("hidden".to_string(), "true".to_string());
    let cmd = Command::Observe(opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Scan(s) = req {
        assert!(s.include_hidden);
    } else {
        panic!("Wrong request type");
    }

    // Test combined options
    let mut opts = HashMap::new();
    opts.insert("near".to_string(), "Submit".to_string());
    opts.insert("viewport".to_string(), "true".to_string());
    opts.insert("max".to_string(), "100".to_string());
    let cmd = Command::Observe(opts);
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Scan(s) = req {
        assert_eq!(s.near, Some("Submit".to_string()));
        assert!(s.viewport_only);
        assert_eq!(s.max_elements, Some(100));
    } else {
        panic!("Wrong request type");
    }
}
