use oryn_core::command::{Command, ExtractSource, Target, WaitCondition};
use oryn_core::protocol::ScannerRequest;
use oryn_core::translator::translate;
use std::collections::HashMap;

#[test]
fn test_translate_click() {
    let cmd = Command::Click(Target::Id(5), HashMap::new());
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Click(c) = req {
        assert_eq!(c.id, 5);
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
    let cmd = Command::Storage("clear".to_string());
    let req = translate(&cmd).unwrap();
    if let ScannerRequest::Execute(e) = req {
        assert!(e.script.contains("localStorage.clear()"));
    } else {
        panic!("Wrong request type");
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
