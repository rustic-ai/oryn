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
    let cmd = parse_one("storage clear");
    if let Command::Storage(op) = cmd {
        assert_eq!(op, "clear");
    } else {
        panic!("Parsed wrong command variant");
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
