//! Immutable native text geometry for OS text-client queries. Coordinates are
//! supplied by the platform adapter; no font metrics or characters are invented.
use oryn_common::v4::CssRect;
use oryn_js::accessibility::{TextBox, TextCaret};
use oryn_js::composition::TextRange;
#[derive(Clone, Debug, Default)]
pub(super) struct TextGeometry {
    pub boxes: Vec<TextBox>,
    pub carets: Vec<TextCaret>,
}
impl TextGeometry {
    pub fn first_rect(&self, range: TextRange) -> Option<(TextRange, CssRect)> {
        let end = range.end()?;
        if range.length == 0 {
            return self
                .carets
                .iter()
                .find(|caret| caret.offset == range.start)
                .and_then(|caret| caret.rect)
                .map(|rect| (range, rect));
        }
        let first = self
            .boxes
            .iter()
            .filter(|item| item.start < end && range.start < item.end)
            .min_by_key(|item| item.start)?;
        let mut rect = first.rect;
        let start = range.start.max(first.start);
        let mut actual_end = end.min(first.end);
        for item in self
            .boxes
            .iter()
            .filter(|item| item.line == first.line && item.start < end && range.start < item.end)
        {
            rect = union(rect, item.rect);
            actual_end = actual_end.max(end.min(item.end));
        }
        Some((
            TextRange {
                start,
                length: actual_end.checked_sub(start)?,
            },
            rect,
        ))
    }
    pub fn index_at(&self, x: f64, y: f64, preferred: u32) -> Option<u32> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        self.carets
            .iter()
            .filter_map(|caret| {
                let rect = caret.rect?;
                let dx = (rect.x - x).max(0.).max(x - rect.right());
                let dy = (rect.y - y).max(0.).max(y - rect.bottom());
                let distance = dx * dx + dy * dy;
                distance.is_finite().then_some((caret.offset, distance))
            })
            .min_by(|a, b| {
                a.1.total_cmp(&b.1)
                    .then_with(|| (a.0 != preferred).cmp(&(b.0 != preferred)))
                    .then_with(|| a.0.cmp(&b.0))
            })
            .map(|(offset, _)| offset)
    }
}
fn union(a: CssRect, b: CssRect) -> CssRect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    CssRect {
        x,
        y,
        width: a.right().max(b.right()) - x,
        height: a.bottom().max(b.bottom()) - y,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rect(x: f64, y: f64, width: f64, height: f64) -> CssRect {
        CssRect {
            x,
            y,
            width,
            height,
        }
    }
    #[test]
    fn first_range_rectangle_stops_at_native_line_and_preserves_utf16() {
        let geometry = TextGeometry {
            boxes: vec![
                TextBox {
                    start: 0,
                    end: 2,
                    line: 7,
                    rect: rect(10., 20., 12., 16.),
                },
                TextBox {
                    start: 2,
                    end: 3,
                    line: 7,
                    rect: rect(22., 20., 8., 16.),
                },
                TextBox {
                    start: 3,
                    end: 4,
                    line: 8,
                    rect: rect(10., 4., 8., 16.),
                },
            ],
            carets: vec![],
        };
        let (range, bounds) = geometry
            .first_rect(TextRange {
                start: 1,
                length: 3,
            })
            .unwrap();
        assert_eq!(
            range,
            TextRange {
                start: 1,
                length: 2
            }
        );
        assert_eq!(bounds, rect(10., 20., 20., 16.));
    }
    #[test]
    fn nearest_caret_uses_both_axes_and_native_bidi_affinity() {
        let geometry = TextGeometry {
            boxes: vec![],
            carets: vec![
                TextCaret {
                    offset: 0,
                    x: 10.,
                    y: 0.,
                    line: 1,
                    rect: Some(rect(10., 0., 12., 0.)),
                },
                TextCaret {
                    offset: 2,
                    x: 10.,
                    y: 20.,
                    line: 1,
                    rect: Some(rect(10., 20., 12., 0.)),
                },
                TextCaret {
                    offset: 5,
                    x: 10.,
                    y: 20.,
                    line: 1,
                    rect: Some(rect(10., 20., 12., 0.)),
                },
            ],
        };
        assert_eq!(geometry.index_at(15., 18., 5), Some(5));
        assert_eq!(geometry.index_at(15., 1., 5), Some(0));
        assert_eq!(
            geometry
                .first_rect(TextRange {
                    start: 2,
                    length: 0
                })
                .unwrap()
                .1,
            rect(10., 20., 12., 0.)
        );
        assert_eq!(geometry.index_at(f64::NAN, 0., 0), None);
    }
}
