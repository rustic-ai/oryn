use oryn_core::intent::builtin;

#[test]
fn test_login_intent_definition() {
    let def = builtin::login::definition();
    assert_eq!(def.name, "login");
    // Ensure triggers are present
    assert!(!def.triggers.patterns.is_empty() || !def.triggers.keywords.is_empty());
    // Ensure parameters are defined
    assert!(def.parameters.iter().any(|p| p.name == "username"));
    assert!(def.parameters.iter().any(|p| p.name == "password"));
    // Ensure steps exist
    assert!(!def.steps.is_empty());
    // Ensure tier is BuiltIn
    assert_eq!(def.tier, oryn_core::intent::definition::IntentTier::BuiltIn);
}

#[test]
fn test_search_intent_definition() {
    let def = builtin::search::definition();
    assert_eq!(def.name, "search");
    assert!(!def.triggers.keywords.is_empty());
    assert!(def.parameters.iter().any(|p| p.name == "query"));
    assert!(!def.steps.is_empty());
}

#[test]
fn test_accept_cookies_intent_definition() {
    let def = builtin::accept_cookies::definition();
    assert_eq!(def.name, "accept_cookies");
    assert!(!def.steps.is_empty());
}

#[test]
fn test_dismiss_popups_intent_definition() {
    let def = builtin::dismiss_popups::definition();
    assert_eq!(def.name, "dismiss_popups");
    assert!(!def.steps.is_empty());
}

#[test]
fn test_fill_form_intent_definition() {
    let def = builtin::fill_form::definition();
    assert_eq!(def.name, "fill_form");
    assert!(def.parameters.iter().any(|p| p.name == "data"));
}

#[test]
fn test_submit_form_intent_definition() {
    let def = builtin::submit_form::definition();
    assert_eq!(def.name, "submit_form");
    assert!(!def.steps.is_empty());
}

#[test]
fn test_scroll_to_intent_definition() {
    let def = builtin::scroll_to::definition();
    assert_eq!(def.name, "scroll_to");
    assert!(def.parameters.iter().any(|p| p.name == "target"));
}

#[test]
fn test_logout_intent_definition() {
    let def = builtin::logout::definition();
    assert_eq!(def.name, "logout");
    assert!(!def.steps.is_empty());
}
