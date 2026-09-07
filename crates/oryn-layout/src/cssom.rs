//! CSSOM declaration operations backed by the same Stylo parser as layout.
use style::{
    context::QuirksMode,
    properties::declaration_block::{
        Importance, SourcePropertyDeclarationUpdate, parse_one_declaration_into,
    },
    properties::{PropertyId, SourcePropertyDeclaration, parse_style_attribute},
    stylesheets::{CssRuleType, Origin, UrlExtraData},
};
use style_traits::ParsingMode;

/// A CSSOM rule view produced by Stylo, including its actual rule kind.
/// The binding layer assigns identity; it must not infer kinds or declaration
/// boundaries by splitting the serialized CSS text.
#[derive(Debug, PartialEq, Eq)]
pub struct RuleView {
    pub rule_type: u16,
    pub css_text: String,
    pub selector_text: Option<String>,
    pub declaration_text: Option<String>,
}

pub fn read_stylesheet(
    source: &str,
    base_url: &str,
    quirks: bool,
    allow_imports: bool,
) -> Result<Vec<RuleView>, String> {
    use cssparser::ToCss;
    use style::{
        media_queries::MediaList,
        servo_arc::Arc,
        shared_lock::{SharedRwLock, ToCssWithGuard},
        stylesheets::{AllowImportRules, CssRule, Stylesheet, StylesheetInDocument},
    };
    let lock = SharedRwLock::new();
    let sheet = Stylesheet::from_str(
        source,
        UrlExtraData::from(url::Url::parse(base_url).map_err(|error| error.to_string())?),
        Origin::Author,
        Arc::new(lock.wrap(MediaList::empty())),
        lock.clone(),
        None,
        None,
        if quirks {
            QuirksMode::Quirks
        } else {
            QuirksMode::NoQuirks
        },
        if allow_imports {
            AllowImportRules::Yes
        } else {
            AllowImportRules::No
        },
    );
    let guard = sheet.shared_lock.read();
    let contents = sheet.contents(&guard);
    contents
        .rules
        .read_with(&guard)
        .0
        .iter()
        .map(|rule| {
            let (selector_text, declaration_text) = if let CssRule::Style(rule) = rule {
                let rule = rule.read_with(&guard);
                let mut declarations = String::new();
                rule.block
                    .read_with(&guard)
                    .to_css(&mut declarations)
                    .map_err(|error| error.to_string())?;
                (Some(rule.selectors.to_css_string()), Some(declarations))
            } else {
                (None, None)
            };
            Ok(RuleView {
                rule_type: rule.rule_type() as u16,
                css_text: rule.to_css_string(&guard).to_string(),
                selector_text,
                declaration_text,
            })
        })
        .collect()
}

/// Result of a declaration mutation. `None` leaves the attribute untouched,
/// including its original spelling and any mutation-observer bookkeeping.
#[derive(Debug, PartialEq, Eq)]
pub struct DeclarationMutation {
    pub previous_value: String,
    pub css_text: Option<String>,
}

/// Set or remove an authored declaration using CSSOM property parsing rules.
pub fn mutate_declaration(
    css_text: &str,
    property: &str,
    value: &str,
    priority: &str,
    base_url: &str,
    quirks: bool,
) -> Result<DeclarationMutation, String> {
    let mut result = DeclarationMutation {
        previous_value: String::new(),
        css_text: None,
    };
    let Ok(property) = PropertyId::parse_enabled_for_all_content(property) else {
        return Ok(result);
    };
    let url_data = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let quirks_mode = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let mut block =
        parse_style_attribute(css_text, &url_data, None, quirks_mode, CssRuleType::Style);
    block
        .property_value_to_css(&property, &mut result.previous_value)
        .map_err(|e| e.to_string())?;
    if value.is_empty() {
        let Some(first) = block.first_declaration_to_remove(&property) else {
            return Ok(result);
        };
        block.remove_property(&property, first);
    } else {
        let importance = if priority.is_empty() {
            Importance::Normal
        } else if priority.eq_ignore_ascii_case("important") {
            Importance::Important
        } else {
            return Ok(result);
        };
        let mut declarations = SourcePropertyDeclaration::default();
        if parse_one_declaration_into(
            &mut declarations,
            property,
            value,
            Origin::Author,
            &url_data,
            None,
            ParsingMode::DEFAULT,
            quirks_mode,
            CssRuleType::Style,
        )
        .is_err()
        {
            return Ok(result);
        }
        let mut updates = SourcePropertyDeclarationUpdate::default();
        if !block.prepare_for_update(&declarations, importance, &mut updates) {
            return Ok(result);
        }
        block.update(declarations.drain(), importance, &mut updates);
    }
    let mut serialized = String::new();
    block.to_css(&mut serialized).map_err(|e| e.to_string())?;
    result.css_text = Some(serialized);
    Ok(result)
}

/// A serialized declaration block and a property queried from that block.
#[derive(Debug, PartialEq, Eq)]
pub struct DeclarationView {
    pub css_text: String,
    pub names: Vec<String>,
    pub value: String,
    pub important: bool,
}

pub fn read_declaration(
    css_text: &str,
    property: &str,
    base_url: &str,
    quirks: bool,
) -> Result<DeclarationView, String> {
    let url_data = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let quirks = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let block = parse_style_attribute(css_text, &url_data, None, quirks, CssRuleType::Style);
    let mut result = DeclarationView {
        css_text: String::new(),
        names: block
            .declaration_importance_iter()
            .map(|(declaration, _)| declaration.id().name().into_owned())
            .collect(),
        value: String::new(),
        important: false,
    };
    block
        .to_css(&mut result.css_text)
        .map_err(|e| e.to_string())?;
    if let Ok(property) = PropertyId::parse_enabled_for_all_content(property) {
        block
            .property_value_to_css(&property, &mut result.value)
            .map_err(|e| e.to_string())?;
        result.important = block.property_priority(&property).important();
    }
    Ok(result)
}

/// Supported property spellings and their canonical (unaliasing) targets.
/// Both enumeration and enablement come from Stylo's generated property table.
pub fn supported_properties() -> Vec<(String, String)> {
    style::properties::NonCustomPropertyId::iter()
        .filter_map(|id| {
            PropertyId::parse_enabled_for_all_content(id.name())
                .ok()
                .map(|_| (id.name().to_owned(), id.unaliased().name().to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stylesheet_replacement_discards_imports_using_the_parser() {
        let rules = read_stylesheet(
            r#"@import url('ignored.css'); @\69mport 'also-ignored.css'; div { color: red; }"#,
            "https://example.test/main.css",
            false,
            false,
        )
        .unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector_text.as_deref(), Some("div"));
    }

    #[test]
    fn stylesheet_views_use_parser_boundaries_and_declaration_validation() {
        let rules = read_stylesheet(
            r#"/* } */ .a, .b { --text: "};:"; width: -1px; margin: 1px 2px; }
               @media (min-width: 10px) { .nested { color: red; } }
               #last { height: 3px; }"#,
            "https://example.test/styles/main.css",
            false,
            true,
        )
        .unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].rule_type, CssRuleType::Style as u16);
        assert_eq!(rules[0].selector_text.as_deref(), Some(".a, .b"));
        let declaration = read_declaration(
            rules[0].declaration_text.as_deref().unwrap(),
            "width",
            "about:blank",
            false,
        )
        .unwrap();
        assert_eq!(declaration.value, "");
        assert!(declaration.css_text.contains("};:"));
        assert_eq!(rules[1].rule_type, CssRuleType::Media as u16);
        assert!(rules[1].selector_text.is_none());
        assert!(rules[1].declaration_text.is_none());
        assert!(rules[1].css_text.contains(".nested"));
        assert_eq!(rules[2].selector_text.as_deref(), Some("#last"));
    }

    #[test]
    fn readback_expands_shorthands_and_preserves_token_boundaries() {
        let view = read_declaration(
            "margin: 1px 2px !important; --Case: 'a;b:c'; width: -1px",
            "margin-left",
            "about:blank",
            false,
        )
        .unwrap();
        assert_eq!(view.value, "2px");
        assert!(view.important);
        assert_eq!(
            view.names,
            [
                "margin-top",
                "margin-right",
                "margin-bottom",
                "margin-left",
                "--Case"
            ]
        );
        let custom = read_declaration(&view.css_text, "--Case", "about:blank", false).unwrap();
        assert!(custom.value.contains("a;b:c"));
        assert_eq!(
            read_declaration(&view.css_text, "--case", "about:blank", false)
                .unwrap()
                .value,
            ""
        );
        assert!(!view.names.iter().any(|name| name == "width"));
    }

    #[test]
    fn invalid_and_absent_mutations_preserve_attribute() {
        for (property, value) in [("width", "-100px"), ("width", ""), ("unknown", "1px")] {
            let result =
                mutate_declaration("top: 0", property, value, "", "about:blank", false).unwrap();
            assert_eq!(result.css_text, None);
        }
    }

    #[test]
    fn shorthand_removal_and_custom_property_case_use_stylo() {
        let removed = mutate_declaration(
            "margin: 1px 2px; color: red",
            "margin",
            "",
            "invalid",
            "about:blank",
            false,
        )
        .unwrap();
        assert_eq!(removed.previous_value, "1px 2px");
        assert_eq!(removed.css_text.as_deref(), Some("color: red;"));
        let changed = mutate_declaration(
            "--Name: one; --name: two",
            "--Name",
            "three",
            "important",
            "about:blank",
            false,
        )
        .unwrap();
        assert_eq!(changed.previous_value, "one");
        let text = changed.css_text.unwrap();
        assert!(text.contains("--Name: three !important;"));
        assert!(text.contains("--name: two;"));
    }
}
