use oryn_engine::command::{Command, ScrollDirection, Target};
use oryn_engine::parser::Parser;

fn parse_one(input: &str) -> Command {
    let mut parser = Parser::new(input);
    let mut commands = parser.parse().expect("Failed to parse command");
    commands.remove(0)
}

#[test]
fn test_submit_command() {
    let cmd = parse_one("submit #login-form");
    if let Command::Submit(target) = cmd {
        assert_eq!(target, Target::Selector("#login-form".into()));
    } else {
        panic!("Parsed wrong command variant");
    }
}

#[test]
fn test_login_command() {
    let cmd = parse_one("login \"bob@example.com\" \"secret123\"");
    if let Command::Login(user, pass, _) = cmd {
        assert_eq!(user, "bob@example.com");
        assert_eq!(pass, "secret123");
    } else {
        panic!("Parsed wrong command variant");
    }
}

#[test]
fn test_search_command() {
    let cmd = parse_one("search \"funny cats\"");
    if let Command::Search(query, _) = cmd {
        assert_eq!(query, "funny cats");
    } else {
        panic!("Parsed wrong command variant");
    }
}

#[test]
fn test_dismiss_accept() {
    let cmd = parse_one("dismiss popups");
    if let Command::Dismiss(target, _) = cmd {
        assert_eq!(target, Target::Text("popups".to_string()));
    } else {
        panic!("Expected Dismiss command");
    }

    let cmd = parse_one("accept cookies");
    if let Command::Accept(target, _) = cmd {
        assert_eq!(target, Target::Text("cookies".to_string()));
    } else {
        panic!("Expected Accept command");
    }
}

#[test]
fn test_scroll_extended() {
    // scroll until <target>
    let cmd = parse_one("scroll until footer");
    if let Command::ScrollUntil(target, dir, _) = cmd {
        assert_eq!(target, Target::Text("footer".into()));
        assert_eq!(dir, ScrollDirection::Down);
    } else {
        panic!("Parsed wrong command variant");
    }

    // scroll down (generic) - checked via options check
    let cmd = parse_one("scroll down");
    if let Command::Scroll(None, opts) = cmd {
        assert_eq!(opts.get("direction").map(|s| s.as_str()), Some("down"));
    } else {
        panic!("Parsed wrong command variant");
    }
}

#[test]
fn test_modifiers_near() {
    // click "Add" near "Cart"
    let cmd = parse_one("click \"Add\" near \"Cart\"");
    if let Command::Click(target, _) = cmd {
        match target {
            Target::Near { target, anchor } => {
                assert_eq!(*target, Target::Text("Add".into()));
                assert_eq!(*anchor, Target::Text("Cart".into()));
            }
            _ => panic!("Expected Near target"),
        }
    } else {
        panic!("Parsed wrong command variant");
    }
}

#[test]
fn test_modifiers_inside() {
    // click "Submit" inside "Login Form"
    let cmd = parse_one("click \"Submit\" inside \"Login Form\"");
    if let Command::Click(target, _) = cmd {
        match target {
            Target::Inside { target, container } => {
                assert_eq!(*target, Target::Text("Submit".into()));
                assert_eq!(*container, Target::Text("Login Form".into()));
            }
            _ => panic!("Expected Inside target"),
        }
    } else {
        panic!("Parsed wrong command variant");
    }
}

#[test]
fn test_modifiers_chained() {
    // click A near B inside C
    // Should parse as: (A near B) inside C based on loop order
    // "near" consumed first loop -> target = (A near B)
    // "inside" consumed second loop -> target = ((A near B) inside C)

    let cmd = parse_one("click A near B inside C");
    if let Command::Click(target, _) = cmd {
        match target {
            Target::Inside {
                target: inner_target,
                container,
            } => {
                assert_eq!(*container, Target::Text("C".into()));
                match *inner_target {
                    Target::Near {
                        target: a,
                        anchor: b,
                    } => {
                        assert_eq!(*a, Target::Text("A".into()));
                        assert_eq!(*b, Target::Text("B".into()));
                    }
                    _ => panic!("Expected Near inside Inside"),
                }
            }
            _ => panic!("Expected Inside target"),
        }
    } else {
        panic!("Parsed wrong command variant");
    }
}
