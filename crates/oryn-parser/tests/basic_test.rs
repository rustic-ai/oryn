use oryn_parser::{parse, normalize};

#[test]
fn test_parser_basic() {
    let input = r#"
    # Comment
    goto "https://example.com" --timeout 5s
    click "Button" near "Menu"
    wait visible "Footer"
    "#;
    
    let normalized = normalize(input);
    println!("Normalized:\n{}", normalized);
    
    let script = parse(&normalized).expect("Failed to parse");
    println!("Parsed: {:?}", script);
    
    assert_eq!(script.lines.len(), 4); // Comment, goto, click, wait
}
