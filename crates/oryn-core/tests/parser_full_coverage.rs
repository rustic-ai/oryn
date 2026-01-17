use oryn_core::command::*;
use oryn_core::parser::Parser;
use std::collections::HashMap;

fn parse_one(input: &str) -> Command {
    let mut parser = Parser::new(input);
    let mut commands = parser.parse().expect("Failed to parse command");
    commands.remove(0)
}

#[test]
fn test_storage_command() {
    // Test storage clear
    let cmd = parse_one("storage clear");
    if let Command::Storage(StorageAction::Clear { storage_type }) = cmd {
        assert_eq!(storage_type, StorageType::Both);
    } else {
        panic!("Parsed wrong command variant for 'storage clear'");
    }

    // Test storage local clear
    let cmd = parse_one("storage local clear");
    if let Command::Storage(StorageAction::Clear { storage_type }) = cmd {
        assert_eq!(storage_type, StorageType::Local);
    } else {
        panic!("Parsed wrong command variant for 'storage local clear'");
    }

    // Test storage get
    let cmd = parse_one("storage get \"authToken\"");
    if let Command::Storage(StorageAction::Get { storage_type, key }) = cmd {
        assert_eq!(storage_type, StorageType::Both);
        assert_eq!(key, "authToken");
    } else {
        panic!("Parsed wrong command variant for 'storage get'");
    }

    // Test storage session set
    let cmd = parse_one("storage session set \"theme\" \"dark\"");
    if let Command::Storage(StorageAction::Set {
        storage_type,
        key,
        value,
    }) = cmd
    {
        assert_eq!(storage_type, StorageType::Session);
        assert_eq!(key, "theme");
        assert_eq!(value, "dark");
    } else {
        panic!("Parsed wrong command variant for 'storage session set'");
    }

    // Test storage list
    let cmd = parse_one("storage list");
    if let Command::Storage(StorageAction::List { storage_type }) = cmd {
        assert_eq!(storage_type, StorageType::Both);
    } else {
        panic!("Parsed wrong command variant for 'storage list'");
    }
}

#[test]
fn test_modifiers_full_set() {
    // after
    let cmd = parse_one("click A after B");
    if let Command::Click(target, _) = cmd {
        match target {
            Target::After { target, anchor } => {
                assert_eq!(*target, Target::Text("A".into()));
                assert_eq!(*anchor, Target::Text("B".into()));
            }
            _ => panic!("Expected After target"),
        }
    } else {
        panic!("Wrong command");
    }

    // before
    let cmd = parse_one("click A before B");
    if let Command::Click(target, _) = cmd {
        match target {
            Target::Before { target, anchor } => {
                assert_eq!(*target, Target::Text("A".into()));
                assert_eq!(*anchor, Target::Text("B".into()));
            }
            _ => panic!("Expected Before target"),
        }
    } else {
        panic!("Wrong command");
    }

    // contains
    let cmd = parse_one("click div contains \"Hello\"");
    if let Command::Click(target, _) = cmd {
        match target {
            Target::Contains { target, content } => {
                assert_eq!(*target, Target::Text("div".into()));
                assert_eq!(*content, Target::Text("Hello".into()));
            }
            _ => panic!("Expected Contains target"),
        }
    } else {
        panic!("Wrong command");
    }
}

#[test]
fn test_missing_basic_commands() {
    // Navigation
    assert_eq!(parse_one("back"), Command::Back);
    assert_eq!(parse_one("forward"), Command::Forward);
    assert_eq!(parse_one("refresh"), Command::Refresh(HashMap::new()));
    assert_eq!(parse_one("url"), Command::Url);

    // Observation
    assert_eq!(parse_one("observe"), Command::Observe(HashMap::new()));
    assert_eq!(parse_one("html"), Command::Html(HashMap::new()));
    assert_eq!(parse_one("title"), Command::Title);
    assert_eq!(parse_one("screenshot"), Command::Screenshot(HashMap::new()));

    // Actions
    assert_eq!(parse_one("hover 5"), Command::Hover(Target::Id(5)));
    assert_eq!(
        parse_one("focus \"Input\""),
        Command::Focus(Target::Text("Input".into()))
    );

    // Complex hover with modifiers (why not)
    // hover "Menu" after "Header"
    let cmd = parse_one("hover \"Menu\" after \"Header\"");
    if let Command::Hover(target) = cmd {
        match target {
            Target::After { target, anchor } => {
                assert_eq!(*target, Target::Text("Menu".into()));
                assert_eq!(*anchor, Target::Text("Header".into()));
            }
            _ => panic!("Expected After target"),
        }
    } else {
        panic!("Wrong command");
    }
}

#[test]
fn test_click_options() {
    // Test --force option
    let cmd = parse_one("click 5 --force");
    if let Command::Click(target, opts) = cmd {
        assert_eq!(target, Target::Id(5));
        assert_eq!(opts.get("force"), Some(&"true".to_string()));
    } else {
        panic!("Wrong command");
    }

    // Test --double option
    let cmd = parse_one("click 5 --double");
    if let Command::Click(_, opts) = cmd {
        assert_eq!(opts.get("double"), Some(&"true".to_string()));
    } else {
        panic!("Wrong command");
    }

    // Test --right option
    let cmd = parse_one("click 5 --right");
    if let Command::Click(_, opts) = cmd {
        assert_eq!(opts.get("right"), Some(&"true".to_string()));
    } else {
        panic!("Wrong command");
    }

    // Test --middle option
    let cmd = parse_one("click 5 --middle");
    if let Command::Click(_, opts) = cmd {
        assert_eq!(opts.get("middle"), Some(&"true".to_string()));
    } else {
        panic!("Wrong command");
    }

    // Test combined options
    let cmd = parse_one("click 5 --force --double");
    if let Command::Click(_, opts) = cmd {
        assert_eq!(opts.get("force"), Some(&"true".to_string()));
        assert_eq!(opts.get("double"), Some(&"true".to_string()));
    } else {
        panic!("Wrong command");
    }
}

#[test]
fn test_type_options() {
    // Test --enter option
    let cmd = parse_one("type email \"test@example.com\" --enter");
    if let Command::Type(target, text, opts) = cmd {
        assert_eq!(target, Target::Role("email".into()));
        assert_eq!(text, "test@example.com");
        assert_eq!(opts.get("enter"), Some(&"true".to_string()));
    } else {
        panic!("Wrong command");
    }

    // Test --append option
    let cmd = parse_one("type 3 \"more text\" --append");
    if let Command::Type(target, text, opts) = cmd {
        assert_eq!(target, Target::Id(3));
        assert_eq!(text, "more text");
        assert_eq!(opts.get("append"), Some(&"true".to_string()));
    } else {
        panic!("Wrong command");
    }

    // Test --delay option with value
    let cmd = parse_one("type 3 \"slow typing\" --delay 100");
    if let Command::Type(_, _, opts) = cmd {
        assert_eq!(opts.get("delay"), Some(&"100".to_string()));
    } else {
        panic!("Wrong command");
    }

    // Test combined options
    let cmd = parse_one("type 5 \"hello\" --append --enter --delay 50");
    if let Command::Type(_, _, opts) = cmd {
        assert_eq!(opts.get("append"), Some(&"true".to_string()));
        assert_eq!(opts.get("enter"), Some(&"true".to_string()));
        assert_eq!(opts.get("delay"), Some(&"50".to_string()));
    } else {
        panic!("Wrong command");
    }
}

#[test]
fn test_observe_options() {
    // Test --near option
    let cmd = parse_one("observe --near \"Login\"");
    if let Command::Observe(opts) = cmd {
        assert_eq!(opts.get("near"), Some(&"Login".to_string()));
    } else {
        panic!("Wrong command");
    }

    // Test --viewport option
    let cmd = parse_one("observe --viewport");
    if let Command::Observe(opts) = cmd {
        assert_eq!(opts.get("viewport"), Some(&"true".to_string()));
    } else {
        panic!("Wrong command");
    }

    // Test --max option with value
    let cmd = parse_one("observe --max 50");
    if let Command::Observe(opts) = cmd {
        assert_eq!(opts.get("max"), Some(&"50".to_string()));
    } else {
        panic!("Wrong command");
    }

    // Test --full option
    let cmd = parse_one("observe --full");
    if let Command::Observe(opts) = cmd {
        assert_eq!(opts.get("full"), Some(&"true".to_string()));
    } else {
        panic!("Wrong command");
    }

    // Test combined options
    let cmd = parse_one("observe --near \"Submit\" --viewport --max 100");
    if let Command::Observe(opts) = cmd {
        assert_eq!(opts.get("near"), Some(&"Submit".to_string()));
        assert_eq!(opts.get("viewport"), Some(&"true".to_string()));
        assert_eq!(opts.get("max"), Some(&"100".to_string()));
    } else {
        panic!("Wrong command");
    }
}
