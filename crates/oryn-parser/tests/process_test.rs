use oryn_parser::{process, resolver::ResolverContext};
use oryn_common::protocol::{Element, ScanResult, PageInfo, ScanStats, ViewportInfo, ScrollInfo, ElementState, Rect, Action, ScannerAction};
use std::collections::HashMap;

fn make_context() -> ResolverContext {
    let elements = vec![
        Element {
            id: 1,
            element_type: "button".into(),
            role: None,
            text: Some("Submit".into()),
            label: None,
            value: None,
            placeholder: None,
            selector: "#submit".into(),
            xpath: None,
            rect: Rect { x:0.0, y:0.0, width:100.0, height:30.0 },
            attributes: HashMap::new(),
            state: ElementState::default(),
            children: vec![],
        }
    ];
    let scan = ScanResult {
        page: PageInfo {
            url: "http://test.com".into(),
            title: "Test".into(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements,
        stats: ScanStats { total:1, scanned:1 },
        patterns: None,
        changes: None,
        available_intents: None,
    };
    ResolverContext::new(&scan)
}

#[test]
fn test_process_click() {
    let ctx = make_context();
    let actions = process("click 'Submit'", &ctx).expect("Process failed");
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        Action::Scanner(ScannerAction::Click(req)) => {
            assert_eq!(req.id, 1);
        }
        _ => panic!("Expected Click ScannerAction, got {:?}", actions[0]),
    }
}

#[test]
fn test_process_multiple() {
    let ctx = make_context();
    // Assuming normalizer handles newlines/semicolons if supported by vectors logic?
    // vectors_test logic was parse/normalize vector.
    // process function uses parse() which parses strictly according to grammar
    // If grammar supports newlines between commands, this works.
    let input = "click 'Submit'\nwait visible 'Submit'";
    // wait visible 'Submit' resolves to ID 1?
    // Wait command implementation uses target_to_selector which extracts ID if resolved.
    
    let actions = process(input, &ctx).expect("Process failed");
    assert_eq!(actions.len(), 2);
    
    match &actions[0] {
        Action::Scanner(ScannerAction::Click(req)) => assert_eq!(req.id, 1),
        _ => panic!("Expected Click"),
    }
    
    match &actions[1] {
        Action::Scanner(ScannerAction::Wait(req)) => {
            // Wait target resolution:
            // resolve_target resolves 'Submit' to ID 1.
            // translate Wait(Visible(Target(Id(1)))) converts to "visible", selector="1" (via target_to_selector).
            // WaitRequest target is Option<String>.
            assert_eq!(req.condition, "visible");
            assert_eq!(req.target.as_deref(), Some("1"));
        }
        _ => panic!("Expected Wait"),
    }
}
