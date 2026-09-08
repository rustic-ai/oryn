//! Media-list transactions preserve the enclosing native stylesheet context.
use super::*;

#[derive(Clone, Copy)]
pub enum MediaListEdit<'a> {
    Replace(&'a str),
    Append(&'a str),
    Delete(&'a str),
}

pub struct MediaListMutation {
    pub rules: Option<Vec<RuleView>>,
    pub not_found: bool,
}

pub fn mutate_media_list(
    source: &str,
    path: &[usize],
    edit: MediaListEdit<'_>,
    base_url: &str,
    quirks: bool,
) -> Result<MediaListMutation, String> {
    use style::{
        media_queries::{MediaList, MediaQuery},
        parser::ParserContext,
        servo_arc::Arc,
        shared_lock::SharedRwLock,
        stylesheets::{AllowImportRules, CssRule, Stylesheet, StylesheetInDocument},
    };
    let url_data = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let quirks = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let lock = SharedRwLock::new();
    let loader = CssomImportLoader {
        base_url: url_data.clone(),
        quirks,
    };
    let sheet = Stylesheet::from_str(
        source,
        url_data.clone(),
        Origin::Author,
        Arc::new(lock.wrap(MediaList::empty())),
        lock.clone(),
        Some(&loader),
        None,
        quirks,
        AllowImportRules::Yes,
    );
    let media = {
        let guard = lock.read();
        let mut rules = sheet.contents(&guard).rules.read_with(&guard).0.as_slice();
        let (&last, ancestors) = path.split_last().ok_or("Empty CSS rule path")?;
        for &index in ancestors {
            let rule = rules.get(index).ok_or("CSS rule path out of bounds")?;
            if matches!(rule, CssRule::Import(_)) {
                return Err("Import sheet requires its own mutation context".into());
            }
            rules = rule.children(&guard);
        }
        match rules.get(last).ok_or("CSS rule path out of bounds")? {
            CssRule::Media(rule) => rule.media_queries.clone(),
            _ => return Err("CSS rule has no media list".into()),
        }
    };
    let mut context = ParserContext::new(
        Origin::Author,
        &url_data,
        Some(CssRuleType::Media),
        ParsingMode::DEFAULT,
        quirks,
        Default::default(),
        None,
        None,
        Default::default(),
    );
    let mut not_found = false;
    let changed = {
        let mut guard = lock.write();
        let media = media.write_with(&mut guard);
        match edit {
            MediaListEdit::Replace(text) => {
                let mut input = cssparser::ParserInput::new(text);
                *media = MediaList::parse(&mut context, &mut cssparser::Parser::new(&mut input));
                true
            }
            MediaListEdit::Append(text) | MediaListEdit::Delete(text) => {
                let mut input = cssparser::ParserInput::new(text);
                let mut parser = cssparser::Parser::new(&mut input);
                if parser
                    .parse_entirely(|input| MediaQuery::parse(&context, input))
                    .is_err()
                {
                    false
                } else if matches!(edit, MediaListEdit::Append(_)) {
                    media.append_medium(&context, text)
                } else {
                    let deleted = media.delete_medium(&context, text);
                    not_found = !deleted;
                    deleted
                }
            }
        }
    };
    let rules = if changed {
        let guard = lock.read();
        Some(
            sheet
                .contents(&guard)
                .rules
                .read_with(&guard)
                .0
                .iter()
                .map(|rule| rule_view(rule, &guard))
                .collect::<Result<Vec<_>, _>>()?,
        )
    } else {
        None
    };
    Ok(MediaListMutation { rules, not_found })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_mutation_preserves_nested_rules_and_namespace_context() {
        let source = "@namespace svg url(http://www.w3.org/2000/svg); @supports (color: red) { @media screen { svg|rect { color: red; } } } aside {}";
        let base = "https://example.test/style.css";
        let result = mutate_media_list(
            source,
            &[1, 0],
            MediaListEdit::Replace("print, (width > 10px)"),
            base,
            false,
        )
        .unwrap();
        assert!(!result.not_found);
        let rules = result.rules.unwrap();
        let media = &rules[1].children[0];
        assert_eq!(media.media_queries, ["print", "(width > 10px)"]);
        assert_eq!(media.children[0].selector_text.as_deref(), Some("svg|rect"));
        assert_eq!(rules[0].rule_type, 10);
        assert_eq!(rules[2].selector_text.as_deref(), Some("aside"));
        assert!(rules[1].css_text.contains("@media print, (width > 10px)"));
    }

    #[test]
    fn media_append_delete_and_invalid_input_have_distinct_results() {
        let source = "@media screen { a {} }";
        let base = "https://example.test/style.css";
        let appended = mutate_media_list(source, &[0], MediaListEdit::Append("print"), base, false)
            .unwrap()
            .rules
            .unwrap();
        assert_eq!(appended[0].media_queries, ["screen", "print"]);
        let deleted = mutate_media_list(
            &appended[0].css_text,
            &[0],
            MediaListEdit::Delete("screen"),
            base,
            false,
        )
        .unwrap();
        assert_eq!(deleted.rules.unwrap()[0].media_queries, ["print"]);
        let absent =
            mutate_media_list(source, &[0], MediaListEdit::Delete("print"), base, false).unwrap();
        assert!(absent.not_found && absent.rules.is_none());
        for edit in [
            MediaListEdit::Append("screen, print"),
            MediaListEdit::Delete("???"),
        ] {
            let result = mutate_media_list(source, &[0], edit, base, false).unwrap();
            assert!(!result.not_found && result.rules.is_none());
        }
        let invalid = mutate_media_list(source, &[0], MediaListEdit::Replace("???"), base, false)
            .unwrap()
            .rules
            .unwrap();
        assert_eq!(invalid[0].media_queries, ["not all"]);
    }
}
