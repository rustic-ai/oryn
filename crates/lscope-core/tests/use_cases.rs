use lscope_core::command::*;
use lscope_core::parser::Parser;
use std::collections::HashMap;

fn parse(input: &str) -> Vec<Command> {
    let mut parser = Parser::new(input);
    parser.parse().expect("Failed to parse commands")
}

fn parse_one(input: &str) -> Command {
    let cmds = parse(input);
    assert_eq!(cmds.len(), 1);
    cmds.into_iter().next().unwrap()
}

#[test]
fn test_scenario_research() {
    // 1. goto google.com
    assert_eq!(
        parse_one("goto google.com"),
        Command::GoTo("google.com".to_string())
    );

    // 2. type 1 "environmental impact..."
    match parse_one("type 1 \"environmental impact lithium\"") {
        Command::Type(Target::Id(1), text, _) => assert_eq!(text, "environmental impact lithium"),
        _ => panic!("Expected Type command"),
    }

    // 3. click "Environmental Impacts..."
    assert_eq!(
        parse_one("click \"Environmental Impacts of Lithium-Ion Batteries - EPA\""),
        Command::Click(
            Target::Text("Environmental Impacts of Lithium-Ion Batteries - EPA".to_string()),
            HashMap::new()
        )
    );

    // 4. text --selector "article"
    match parse_one("text --selector \"article\"") {
        Command::Text(opts) => {
            assert_eq!(opts.get("selector").map(|s| s.as_str()), Some("article"));
        }
        _ => panic!("Expected Text command"),
    }
}

#[test]
fn test_scenario_ecommerce() {
    // type 1 "men's blue oxford shirt"
    match parse_one("type 1 \"men's blue oxford shirt\"") {
        Command::Type(Target::Id(1), text, _) => assert_eq!(text, "men's blue oxford shirt"),
        _ => panic!("Expected Type"),
    }

    // check "Size: Large"
    assert_eq!(
        parse_one("check \"Size: Large\""),
        Command::Check(Target::Text("Size: Large".to_string()))
    );

    // select 3 "L"
    match parse_one("select 3 \"L\"") {
        Command::Select(Target::Id(3), val) => assert_eq!(val, "L"),
        _ => panic!("Expected Select"),
    }

    // click "Add to Cart"
    assert_eq!(
        parse_one("click \"Add to Cart\""),
        Command::Click(Target::Text("Add to Cart".to_string()), HashMap::new())
    );
}

#[test]
fn test_scenario_travel() {
    // type "From" "SFO"
    // "From" is likely a placeholder or label
    // `type "From" "SFO"` -> Type(Target::Text("From"), "SFO")
    match parse_one("type \"From\" \"SFO\"") {
        Command::Type(Target::Text(t), text, _) => {
            assert_eq!(t, "From");
            assert_eq!(text, "SFO");
        }
        _ => panic!("Expected Type"),
    }

    // click 23
    assert_eq!(
        parse_one("click 23"),
        Command::Click(Target::Id(23), HashMap::new())
    );
}

#[test]
fn test_scenario_github() {
    // click "Settings"
    assert_eq!(
        parse_one("click \"Settings\""),
        Command::Click(Target::Text("Settings".to_string()), HashMap::new())
    );

    // clear 11
    assert_eq!(parse_one("clear 11"), Command::Clear(Target::Id(11)));

    // uncheck "Stop watching..."
    assert_eq!(
        parse_one("uncheck \"Stop watching notifications\""),
        Command::Uncheck(Target::Text("Stop watching notifications".to_string()))
    );
}

#[test]
fn test_scenario_linkedin() {
    // click "Start a post"
    assert_eq!(
        parse_one("click \"Start a post\""),
        Command::Click(Target::Text("Start a post".to_string()), HashMap::new())
    );
}

#[test]
fn test_grammar_edge_cases() {
    // Selectors
    match parse_one("click css(.btn-primary)") {
        Command::Click(Target::Selector(s), _) => assert_eq!(s, ".btn-primary"),
        _ => panic!("Expected Selector"),
    }

    // Wait
    match parse_one("wait visible 5") {
        Command::Wait(WaitCondition::Visible(Target::Id(5)), _) => {}
        _ => panic!("Expected Wait Visible 5"),
    }
    match parse_one("wait url \"https://.*\"") {
        Command::Wait(WaitCondition::Url(u), _) => assert_eq!(u, "https://.*"),
        _ => panic!("Expected Wait Url"),
    }

    // Complex Key Press
    match parse_one("press Control+Enter") {
        Command::Press(k, _) => assert_eq!(k, "Control+Enter"),
        _ => panic!("Expected Press"),
    }

    // Escaping
    match parse_one("type 1 \"He said \\\"Hello\\\"\"") {
        Command::Type(_, t, _) => assert_eq!(t, "He said \"Hello\""),
        _ => panic!("Expected escaped string"),
    }

    // Aliases
    assert_eq!(
        parse_one("navigate to google.com"),
        Command::GoTo("google.com".to_string())
    );

    // Extraction
    match parse_one("extract tables") {
        Command::Extract(ExtractSource::Tables) => {}
        _ => panic!("Expected extract tables"),
    }
    match parse_one("extract css(div > span)") {
        Command::Extract(ExtractSource::Css(s)) => assert_eq!(s, "div > span"),
        _ => panic!("Expected extract css"),
    }
}
