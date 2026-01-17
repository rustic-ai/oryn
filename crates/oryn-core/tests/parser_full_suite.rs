use oryn_core::command::*;
use oryn_core::parser::Parser;
use std::collections::HashMap;

fn parse_one(input: &str) -> Command {
    let mut parser = Parser::new(input);
    let mut commands = parser.parse().expect("Failed to parse command");
    commands.remove(0)
}

#[test]
fn test_parser_wait_expanded() {
    assert_eq!(
        parse_one("wait idle"),
        Command::Wait(WaitCondition::Idle, HashMap::new())
    );
    assert_eq!(
        parse_one("wait hidden 5"),
        Command::Wait(WaitCondition::Hidden(Target::Id(5)), HashMap::new())
    );
    assert_eq!(
        parse_one("wait exists css(.spinner)"),
        Command::Wait(WaitCondition::Exists(".spinner".into()), HashMap::new())
    );
    assert_eq!(
        parse_one("wait gone \"#overlay\""),
        Command::Wait(WaitCondition::Gone("#overlay".into()), HashMap::new())
    );
    assert_eq!(
        parse_one("wait url \"success\""),
        Command::Wait(WaitCondition::Url("success".into()), HashMap::new())
    );
}

#[test]
fn test_parser_actions_missing() {
    assert_eq!(parse_one("clear 5"), Command::Clear(Target::Id(5)));
    assert_eq!(parse_one("submit 5"), Command::Submit(Target::Id(5)));
}

#[test]
fn test_parser_extract_expanded() {
    assert_eq!(
        parse_one("extract links"),
        Command::Extract(ExtractSource::Links)
    );
    assert_eq!(
        parse_one("extract images"),
        Command::Extract(ExtractSource::Images)
    );
    assert_eq!(
        parse_one("extract tables"),
        Command::Extract(ExtractSource::Tables)
    );
    assert_eq!(
        parse_one("extract meta"),
        Command::Extract(ExtractSource::Meta)
    );
    assert_eq!(
        parse_one("extract css(.item)"),
        Command::Extract(ExtractSource::Css(".item".into()))
    );
}

#[test]
fn test_parser_cookies_expanded() {
    assert_eq!(
        parse_one("cookies list"),
        Command::Cookies(CookieAction::List)
    );
    assert_eq!(
        parse_one("cookies get session"),
        Command::Cookies(CookieAction::Get("session".into()))
    );
    assert_eq!(
        parse_one("cookies delete session"),
        Command::Cookies(CookieAction::Delete("session".into()))
    );
    assert_eq!(
        parse_one("cookies set user bob"),
        Command::Cookies(CookieAction::Set("user".into(), "bob".into()))
    );
}

#[test]
fn test_parser_tabs_expanded() {
    assert_eq!(parse_one("tabs"), Command::Tabs(TabAction::List));
    assert_eq!(parse_one("tabs list"), Command::Tabs(TabAction::List));
    assert_eq!(
        parse_one("tabs new google.com"),
        Command::Tabs(TabAction::New("google.com".into()))
    );
    assert_eq!(
        parse_one("tabs switch \"tab-1\""),
        Command::Tabs(TabAction::Switch("tab-1".into()))
    );
    assert_eq!(
        parse_one("tabs close \"tab-1\""),
        Command::Tabs(TabAction::Close("tab-1".into()))
    );
}

#[test]
fn test_parser_scroll_expanded() {
    // scroll [target] [direction]
    let cmd = parse_one("scroll 5 up");
    if let Command::Scroll(Some(Target::Id(5)), opts) = cmd {
        assert_eq!(opts.get("direction").unwrap(), "up");
    } else {
        panic!("Wrong scroll command: {:?}", cmd);
    }

    let cmd = parse_one("scroll left");
    if let Command::Scroll(None, opts) = cmd {
        assert_eq!(opts.get("direction").unwrap(), "left");
    } else {
        panic!("Wrong scroll command: {:?}", cmd);
    }
}

#[test]
fn test_parser_pdf() {
    assert_eq!(
        parse_one("pdf \"report.pdf\""),
        Command::Pdf("report.pdf".into())
    );
}

#[test]
fn test_parser_aliases() {
    assert_eq!(parse_one("see"), Command::Observe(HashMap::new()));
    assert_eq!(parse_one("snap"), Command::Screenshot(HashMap::new()));
    assert_eq!(parse_one("reload"), Command::Refresh(HashMap::new()));
    assert_eq!(
        parse_one("fill 5 \"text\""),
        Command::Type(Target::Id(5), "text".into(), HashMap::new())
    );
}

#[test]
fn test_parser_selectors_xpath() {
    match parse_one("click xpath(//button)") {
        Command::Click(Target::Selector(s), _) => assert_eq!(s, "//button"),
        _ => panic!("Expected xpath selector"),
    }
}

#[test]
fn test_parser_composite_with_options() {
    let cmd = parse_one("login \"user\" \"pass\" --domain example.com");
    if let Command::Login(u, p, opts) = cmd {
        assert_eq!(u, "user");
        assert_eq!(p, "pass");
        assert_eq!(opts.get("domain").unwrap(), "example.com");
    } else {
        panic!("Wrong login command");
    }

    let cmd = parse_one("search \"oryn\" --engine google");
    if let Command::Search(q, opts) = cmd {
        assert_eq!(q, "oryn");
        assert_eq!(opts.get("engine").unwrap(), "google");
    } else {
        panic!("Wrong search command");
    }
}
