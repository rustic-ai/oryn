//! Consume native shaping provenance without independently applying CSS text rules.
use oryn_common::v4::{ResolvedTextRun, ResolvedTextSpan};
use std::collections::BTreeMap;
pub(crate) struct ResolvedNodeText {
    pub units: Vec<u16>,
    pub starts: Vec<u32>,
    pub ends: Vec<u32>,
    pub offsets: Vec<usize>,
}
struct Piece {
    span: ResolvedTextSpan,
    units: Vec<u16>,
}
pub(crate) fn resolved_nodes(
    runs: &[ResolvedTextRun],
) -> Result<BTreeMap<u64, ResolvedNodeText>, String> {
    let mut pieces: BTreeMap<u64, Vec<Piece>> = BTreeMap::new();
    let mut aliases: BTreeMap<u64, Vec<ResolvedTextSpan>> = BTreeMap::new();
    let mut bytes = 0usize;
    let mut count = 0usize;
    for run in runs {
        bytes = bytes
            .checked_add(run.text.len())
            .ok_or("resolved AX text budget overflow")?;
        if bytes > 8 * 1024 * 1024 {
            return Err("resolved AX text byte budget exceeded".into());
        }
        for (alias, span) in run
            .spans
            .iter()
            .map(|span| (false, span))
            .chain(run.aliases.iter().map(|span| (true, span)))
        {
            count += 1;
            if count > 1_048_576 {
                return Err("resolved AX span budget exceeded".into());
            }
            let Some(node) = span.source_node else {
                continue;
            };
            if span.source_start > span.source_end || span.source_end > 1_048_576 {
                return Err("resolved AX source offset invalid".into());
            }
            let Some(text) = run.text.get(span.rendered_start..span.rendered_end) else {
                return Err("resolved AX byte boundary invalid".into());
            };
            if alias {
                aliases.entry(node).or_default().push(span.clone());
            } else {
                pieces.entry(node).or_default().push(Piece {
                    span: span.clone(),
                    units: text.encode_utf16().collect(),
                });
            }
        }
    }
    // Erased-only nodes remain in the native document stream with zero rendered
    // width; this preserves their DOM endpoint without duplicating the base glyph.
    for (node, spans) in &aliases {
        if !pieces.contains_key(node) {
            pieces.insert(
                *node,
                spans
                    .iter()
                    .map(|span| Piece {
                        span: span.clone(),
                        units: Vec::new(),
                    })
                    .collect(),
            );
        }
    }
    let mut result = BTreeMap::new();
    let mut total = 0usize;
    for (node, mut pieces) in pieces {
        pieces.sort_by_key(|piece| (piece.span.source_start, piece.span.source_end));
        let source_len = pieces
            .iter()
            .map(|piece| piece.span.source_end)
            .chain(
                aliases
                    .get(&node)
                    .into_iter()
                    .flatten()
                    .map(|span| span.source_end),
            )
            .max()
            .unwrap_or(0);
        total = total
            .checked_add(source_len + 1)
            .ok_or("resolved AX mapping budget overflow")?;
        if total > 4_194_304 {
            return Err("resolved AX source mapping budget exceeded".into());
        }
        let mut output = ResolvedNodeText {
            units: Vec::new(),
            starts: vec![0],
            ends: vec![0],
            offsets: vec![usize::MAX; source_len + 1],
        };
        let mut previous: Option<(usize, usize, Vec<u16>)> = None;
        for piece in pieces {
            let span = piece.span;
            if let Some((start, end, units)) = &previous {
                if span.source_start == *start && span.source_end == *end {
                    if piece.units == *units {
                        continue;
                    }
                    return Err("conflicting native AX source runs".into());
                }
                if span.source_start < *end {
                    return Err("overlapping native AX source runs".into());
                }
            }
            let start = output.units.len();
            let length = piece.units.len();
            let source_length = span.source_end - span.source_start;
            if !span.transformed && length != source_length && !piece.units.is_empty() {
                return Err("native AX identity span changed UTF16 length".into());
            }
            if start.saturating_add(length) > 1_048_576 {
                return Err("resolved AX node text budget exceeded".into());
            }
            for offset in span.source_start..=span.source_end {
                output.offsets[offset] = if length == source_length {
                    start + offset - span.source_start
                } else if offset == span.source_end {
                    start + length
                } else {
                    start
                };
            }
            *output.starts.last_mut().unwrap() = span.source_start as u32;
            *output.ends.last_mut().unwrap() = span.source_start as u32;
            if length == 0 {
                *output.starts.last_mut().unwrap() = span.source_end as u32;
                *output.ends.last_mut().unwrap() = span.source_end as u32;
            }
            for index in 1..=length {
                let (before, after) = if length == source_length {
                    let point = (span.source_start + index) as u32;
                    (point, point)
                } else if index == length {
                    (span.source_end as u32, span.source_end as u32)
                } else {
                    (span.source_start as u32, span.source_end as u32)
                };
                output.starts.push(before);
                output.ends.push(after);
            }
            previous = Some((span.source_start, span.source_end, piece.units.clone()));
            output.units.extend(piece.units);
        }
        if let Some(aliases) = aliases.get(&node) {
            for alias in aliases {
                let anchor = output.offsets[..=alias.source_start]
                    .iter()
                    .rev()
                    .copied()
                    .find(|offset| *offset != usize::MAX)
                    .or_else(|| {
                        output.offsets[alias.source_end..]
                            .iter()
                            .copied()
                            .find(|offset| *offset != usize::MAX)
                    })
                    .unwrap_or(0);
                for offset in alias.source_start..=alias.source_end {
                    if output.offsets[offset] == usize::MAX {
                        output.offsets[offset] = anchor;
                    }
                }
                if let Some(end) = output.ends.get_mut(anchor)
                    && *end >= alias.source_start as u32
                    && *end <= alias.source_end as u32
                {
                    *end = alias.source_end as u32;
                }
            }
        }
        result.insert(node, output);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn span(
        node: u64,
        source: std::ops::Range<usize>,
        rendered: std::ops::Range<usize>,
        transformed: bool,
    ) -> ResolvedTextSpan {
        ResolvedTextSpan {
            owner: 1,
            source_node: Some(node),
            source_start: source.start,
            source_end: source.end,
            rendered_start: rendered.start,
            rendered_end: rendered.end,
            transformed,
            atomic: false,
        }
    }
    #[test]
    fn collapse_and_expansion_preserve_both_dom_endpoint_affinities() {
        let runs = vec![ResolvedTextRun {
            layout_node: 1,
            text: "A SS".into(),
            spans: vec![
                span(2, 0..1, 0..1, false),
                span(2, 1..4, 1..2, true),
                span(2, 4..5, 2..4, true),
            ],
            aliases: vec![],
        }];
        let nodes = resolved_nodes(&runs).unwrap();
        let node = &nodes[&2];
        assert_eq!(String::from_utf16(&node.units).unwrap(), "A SS");
        assert_eq!(node.offsets, vec![0, 1, 1, 1, 2, 4]);
        assert_eq!((node.starts[3], node.ends[3]), (4, 5));
    }
    #[test]
    fn erased_only_alias_does_not_duplicate_rendered_base() {
        let nodes = resolved_nodes(&[ResolvedTextRun {
            layout_node: 1,
            text: "I".into(),
            spans: vec![span(2, 0..1, 0..1, false)],
            aliases: vec![span(3, 0..1, 0..1, true)],
        }])
        .unwrap();
        assert_eq!(nodes[&2].units, vec![73]);
        assert!(nodes[&3].units.is_empty());
        assert_eq!(nodes[&3].offsets, vec![0, 0]);
    }
    #[test]
    fn invalid_boundaries_and_conflicting_repeated_runs_fail() {
        let mut run = ResolvedTextRun {
            layout_node: 1,
            text: "😀".into(),
            spans: vec![span(2, 0..2, 0..1, false)],
            aliases: vec![],
        };
        assert!(resolved_nodes(&[run.clone()]).is_err());
        run.spans[0].rendered_end = 4;
        assert!(resolved_nodes(&[run.clone(), run.clone()]).is_ok());
        let mut conflicting = run.clone();
        conflicting.text = "abcd".into();
        assert!(resolved_nodes(&[run, conflicting]).is_err());
    }
}
