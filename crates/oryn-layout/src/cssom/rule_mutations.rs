//! CSSOM list edits against the complete native stylesheet tree.
use super::*;
use style::{
    media_queries::MediaList,
    servo_arc::Arc,
    shared_lock::SharedRwLock,
    stylesheets::{
        AllowImportRules, CssRule, CssRuleTypes, CssRules, RulesMutateError, Stylesheet,
        StylesheetInDocument,
    },
};

#[derive(Debug, PartialEq, Eq)]
pub enum RuleMutationError {
    Syntax,
    IndexSize,
    HierarchyRequest,
    InvalidState,
    InvalidContext(String),
}

impl From<RulesMutateError> for RuleMutationError {
    fn from(error: RulesMutateError) -> Self {
        match error {
            RulesMutateError::Syntax => Self::Syntax,
            RulesMutateError::IndexSize => Self::IndexSize,
            RulesMutateError::HierarchyRequest => Self::HierarchyRequest,
            RulesMutateError::InvalidState => Self::InvalidState,
        }
    }
}

pub enum RuleListEdit<'a> {
    Insert { index: usize, text: &'a str },
    Delete { index: usize },
}

/// Empty `parent_path` targets the stylesheet. Other paths target a grouping,
/// style or page rule's child list. Import sheets and keyframes use separate
/// ownership and mutation APIs, so cannot be traversed through this entry point.
/// The result is published only after a successful native mutation.
pub fn mutate_rule_list(
    source: &str,
    parent_path: &[usize],
    edit: RuleListEdit<'_>,
    base_url: &str,
    quirks: bool,
    allow_imports: bool,
) -> Result<Vec<RuleView>, RuleMutationError> {
    let url_data = UrlExtraData::from(
        url::Url::parse(base_url)
            .map_err(|error| RuleMutationError::InvalidContext(error.to_string()))?,
    );
    let quirks_mode = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let lock = SharedRwLock::new();
    let loader = CssomImportLoader {
        base_url: url_data.clone(),
        quirks: quirks_mode,
    };
    // Existing import metadata must survive edits even when inserting a new
    // import is disallowed by the owning stylesheet's construction policy.
    let sheet = Stylesheet::from_str(
        source,
        url_data,
        Origin::Author,
        Arc::new(lock.wrap(MediaList::empty())),
        lock.clone(),
        Some(&loader),
        None,
        quirks_mode,
        AllowImportRules::Yes,
    );
    let mut list = sheet.contents(&lock.read()).rules.clone();
    let mut containing = CssRuleTypes::default();
    let mut relative = None;
    for &index in parent_path {
        let rule = list
            .read_with(&lock.read())
            .0
            .get(index)
            .cloned()
            .ok_or(RuleMutationError::IndexSize)?;
        let kind = rule.rule_type();
        containing.insert(kind);
        if matches!(kind, CssRuleType::Style | CssRuleType::Scope) {
            relative = Some(kind);
        }
        list = match rule {
            CssRule::Style(rule) => {
                let existing = rule.read_with(&lock.read()).rules.clone();
                match existing {
                    Some(list) => list,
                    None => {
                        let list = CssRules::new(Vec::new(), &lock);
                        rule.write_with(&mut lock.write()).rules = Some(list.clone());
                        list
                    }
                }
            }
            CssRule::Page(rule) => rule.read_with(&lock.read()).rules.clone(),
            CssRule::Media(rule) => rule.rules.clone(),
            CssRule::Supports(rule) => rule.rules.clone(),
            CssRule::Container(rule) => rule.rules.clone(),
            CssRule::Document(rule) => rule.rules.clone(),
            CssRule::LayerBlock(rule) => rule.rules.clone(),
            CssRule::Scope(rule) => rule.rules.clone(),
            CssRule::StartingStyle(rule) => rule.rules.clone(),
            CssRule::AppearanceBase(rule) => rule.rules.clone(),
            _ => return Err(RuleMutationError::HierarchyRequest),
        };
    }
    match edit {
        RuleListEdit::Insert { index, text } => {
            let rule = {
                let guard = lock.read();
                list.read_with(&guard).parse_rule_for_insert(
                    &lock,
                    text,
                    sheet.contents(&guard),
                    index,
                    containing,
                    relative,
                    Some(&loader),
                    if allow_imports {
                        AllowImportRules::Yes
                    } else {
                        AllowImportRules::No
                    },
                )?
            };
            list.write_with(&mut lock.write()).0.insert(index, rule);
        }
        RuleListEdit::Delete { index } => {
            list.write_with(&mut lock.write()).remove_rule(index)?;
        }
    }
    let guard = lock.read();
    sheet
        .contents(&guard)
        .rules
        .read_with(&guard)
        .0
        .iter()
        .map(|rule| rule_view(rule, &guard).map_err(RuleMutationError::InvalidContext))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: &str = "https://example.test/styles/main.css";

    #[test]
    fn grouping_families_share_native_list_mutations() {
        for source in [
            "@media all { p {} }",
            "@supports (display: block) { p {} }",
            "@container (width > 1px) { p {} }",
            "@layer theme { p {} }",
            "@scope (.root) { p {} }",
            "@starting-style { p {} }",
        ] {
            let before = read_stylesheet(source, BASE, false, true).unwrap();
            assert_eq!(before.len(), 1, "{source}");
            let after = mutate_rule_list(
                source,
                &[0],
                RuleListEdit::Insert {
                    index: 1,
                    text: "section { color: blue }",
                },
                BASE,
                false,
                true,
            )
            .unwrap();
            assert_eq!(after[0].rule_type, before[0].rule_type, "{source}");
            assert_eq!(after[0].children.len(), 2, "{source}");
            assert_eq!(after[0].children[0], before[0].children[0], "{source}");
            let restored = mutate_rule_list(
                &after[0].css_text,
                &[0],
                RuleListEdit::Delete { index: 1 },
                BASE,
                false,
                true,
            )
            .unwrap();
            assert_eq!(restored, before, "{source}");
        }
    }

    #[test]
    fn top_level_insert_rejects_misordered_import_and_preserves_import_metadata() {
        let source = "@import url(../base.css); p {}";
        let before = read_stylesheet(source, BASE, false, true).unwrap();
        let after = mutate_rule_list(
            source,
            &[],
            RuleListEdit::Insert {
                index: 1,
                text: "aside {}",
            },
            BASE,
            false,
            true,
        )
        .unwrap();
        assert_eq!(after[0], before[0]);
        assert_eq!(after[2], before[1]);
        assert_eq!(
            mutate_rule_list(
                source,
                &[],
                RuleListEdit::Insert {
                    index: 0,
                    text: "aside {}",
                },
                BASE,
                false,
                true
            ),
            Err(RuleMutationError::HierarchyRequest)
        );
        assert_eq!(
            mutate_rule_list(
                "",
                &[],
                RuleListEdit::Insert {
                    index: 0,
                    text: "@import url(other.css);",
                },
                BASE,
                false,
                false
            ),
            Err(RuleMutationError::Syntax)
        );
    }

    #[test]
    fn nested_insert_keeps_namespace_context_and_unrelated_rules() {
        let source = "@namespace svg url(http://www.w3.org/2000/svg); @media all { svg|rect { color: red } } aside { color: blue }";
        let before = read_stylesheet(source, BASE, false, true).unwrap();
        let after = mutate_rule_list(
            source,
            &[1],
            RuleListEdit::Insert {
                index: 0,
                text: "svg|circle { fill: green }",
            },
            BASE,
            false,
            true,
        )
        .unwrap();
        assert_eq!(after.len(), 3);
        assert_eq!(after[0], before[0]);
        assert_eq!(after[2], before[2]);
        assert_eq!(after[1].children.len(), 2);
        assert_eq!(
            after[1].children[0].selector_text.as_deref(),
            Some("svg|circle")
        );
        assert_eq!(after[1].children[1], before[1].children[0]);
    }

    #[test]
    fn list_edits_enforce_native_error_precedence_and_hierarchy() {
        let source = "@media all { p {} }";
        for (index, text, error) in [
            (2, "???", RuleMutationError::IndexSize),
            (0, "???", RuleMutationError::Syntax),
            (0, "p {} garbage", RuleMutationError::Syntax),
            (0, "p {} q {}", RuleMutationError::Syntax),
            (
                0,
                "@import url(other.css);",
                RuleMutationError::HierarchyRequest,
            ),
            (
                0,
                "@namespace url(http://www.w3.org/1999/xhtml);",
                RuleMutationError::HierarchyRequest,
            ),
        ] {
            assert_eq!(
                mutate_rule_list(
                    source,
                    &[0],
                    RuleListEdit::Insert { index, text },
                    BASE,
                    false,
                    true
                ),
                Err(error),
                "{text} at {index}"
            );
        }
        assert_eq!(
            mutate_rule_list(
                source,
                &[0],
                RuleListEdit::Delete { index: 1 },
                BASE,
                false,
                true
            ),
            Err(RuleMutationError::IndexSize)
        );
    }

    #[test]
    fn nested_delete_preserves_parent_declarations_and_siblings() {
        let source = "article { color: red; @media all { & > b {} & > i {} } } footer {}";
        let before = read_stylesheet(source, BASE, false, true).unwrap();
        let after = mutate_rule_list(
            source,
            &[0, 0],
            RuleListEdit::Delete { index: 0 },
            BASE,
            false,
            true,
        )
        .unwrap();
        assert_eq!(after[0].declaration_text, before[0].declaration_text);
        assert_eq!(after[1], before[1]);
        assert_eq!(
            after[0].children[0].children,
            vec![before[0].children[0].children[1].clone()]
        );
    }

    #[test]
    fn inserting_into_empty_style_list_uses_nesting_context() {
        let after = mutate_rule_list(
            "article { color: red }",
            &[0],
            RuleListEdit::Insert {
                index: 0,
                text: "> b { color: blue }",
            },
            BASE,
            false,
            true,
        )
        .unwrap();
        assert_eq!(after[0].declaration_text.as_deref(), Some("color: red;"));
        assert_eq!(after[0].children.len(), 1);
        assert_eq!(after[0].children[0].selector_text.as_deref(), Some("& > b"));
    }

    #[test]
    fn namespace_delete_is_rejected_while_body_rules_exist() {
        let source = "@namespace svg url(http://www.w3.org/2000/svg); svg|rect {}";
        assert_eq!(
            mutate_rule_list(
                source,
                &[],
                RuleListEdit::Delete { index: 0 },
                BASE,
                false,
                true
            ),
            Err(RuleMutationError::InvalidState)
        );
        let after = mutate_rule_list(
            "@namespace svg url(http://www.w3.org/2000/svg);",
            &[],
            RuleListEdit::Delete { index: 0 },
            BASE,
            false,
            true,
        )
        .unwrap();
        assert!(after.is_empty());
    }
}
