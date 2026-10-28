//! Index native semantic text by DOM identity, never by painted line order.
use oryn_common::v4::ResolvedTextRun;
use std::collections::BTreeMap;

/// CSS-normalized/transformed text before soft-wrap trimming. Generated and
/// atomic layout placeholders have no DOM text contribution.
pub fn semantic_nodes(runs: &[ResolvedTextRun]) -> Result<BTreeMap<u64, String>, String> {
    let mut pieces: BTreeMap<u64, Vec<(usize, usize, String)>> = BTreeMap::new();
    let mut bytes = 0usize;
    let mut spans = 0usize;
    for run in runs {
        let source = run
            .semantic_source
            .as_ref()
            .ok_or("missing semantic text source")?;
        bytes = bytes
            .checked_add(source.text.len())
            .ok_or("semantic text budget overflow")?;
        if bytes > 8 * 1024 * 1024 {
            return Err("semantic text byte budget exceeded".into());
        }
        for span in &source.spans {
            spans += 1;
            if spans > 1_048_576 {
                return Err("semantic text span budget exceeded".into());
            }
            let text = source
                .text
                .get(span.rendered_start..span.rendered_end)
                .ok_or("semantic text byte boundary invalid")?;
            if span.source_start > span.source_end || span.source_end > 1_048_576 {
                return Err("semantic text source offset invalid".into());
            }
            if span.atomic || span.source_node.is_none() {
                continue;
            }
            if !span.transformed
                && text.encode_utf16().count() != span.source_end - span.source_start
            {
                return Err("semantic identity span changed UTF16 length".into());
            }
            pieces.entry(span.source_node.unwrap()).or_default().push((
                span.source_start,
                span.source_end,
                text.to_owned(),
            ));
        }
    }
    let mut result = BTreeMap::new();
    for (node, mut fragments) in pieces {
        fragments.sort_by_key(|piece| (piece.0, piece.1));
        let mut previous: Option<(usize, usize, String)> = None;
        let mut text = String::new();
        for fragment in fragments {
            if let Some(old) = &previous {
                if (old.0, old.1) == (fragment.0, fragment.1) {
                    if old.2 == fragment.2 {
                        continue;
                    }
                    return Err("conflicting semantic text source runs".into());
                }
                if fragment.0 < old.1 {
                    return Err("overlapping semantic text source runs".into());
                }
            }
            text.push_str(&fragment.2);
            previous = Some(fragment);
        }
        result.insert(node, text);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_common::v4::{ResolvedTextSpan, SemanticTextSource};
    fn run(text: &str, start: usize, end: usize) -> ResolvedTextRun {
        ResolvedTextRun {
            layout_node: 9,
            text: "paint is not source".into(),
            spans: vec![],
            aliases: vec![],
            semantic_source: Some(SemanticTextSource {
                text: text.into(),
                spans: vec![ResolvedTextSpan {
                    owner: 7,
                    source_node: Some((3u64 << 32) | 2),
                    rendered_start: 0,
                    rendered_end: text.len(),
                    source_start: start,
                    source_end: end,
                    transformed: false,
                    atomic: false,
                }],
            }),
        }
    }
    #[test]
    fn native_identity_order_and_soft_wrap_spaces_survive() {
        let result = semantic_nodes(&[run("b", 2, 3), run("a ", 0, 2), run("b", 2, 3)]).unwrap();
        assert_eq!(result[&((3u64 << 32) | 2)], "a b");
        assert!(!result.contains_key(&2));
    }
    #[test]
    fn invalid_or_conflicting_projection_fails_closed() {
        assert!(semantic_nodes(&[run("a", 0, 1), run("b", 0, 1)]).is_err());
        assert!(semantic_nodes(&[run("ab", 0, 2), run("c", 1, 2)]).is_err());
        assert!(semantic_nodes(&[run("😀", 0, 1)]).is_err());
        let mut value = run("é", 0, 1);
        value.semantic_source.as_mut().unwrap().spans[0].rendered_start = 1;
        assert!(semantic_nodes(&[value]).is_err());
        let mut value = run("a", 0, 1);
        value.semantic_source = None;
        assert!(semantic_nodes(&[value]).is_err());
    }
    #[test]
    fn transformed_text_and_non_dom_placeholders() {
        let mut value = run("SS", 0, 1);
        value.semantic_source.as_mut().unwrap().spans[0].transformed = true;
        assert_eq!(
            semantic_nodes(&[value.clone()])
                .unwrap()
                .values()
                .next()
                .unwrap(),
            "SS"
        );
        value.semantic_source.as_mut().unwrap().spans[0].atomic = true;
        assert!(semantic_nodes(&[value]).unwrap().is_empty());
    }
}
