use lscope_core::command::{Command, Target, WaitCondition};
use lscope_core::protocol::ScannerRequest;
use lscope_core::translator::translate;
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
        assert!(matches!(s.direction, lscope_core::protocol::ScrollDirection::Up));
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
