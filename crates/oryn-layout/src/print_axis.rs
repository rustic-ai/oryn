//! Fragmentation operates in page block progression. Only geometry used by the
//! break algorithm is mapped; native shaped glyphs keep their physical bases.
use super::*;
#[derive(Clone, Copy)]
pub(super) struct PageAxis {
    pub vertical: bool,
    pub reverse: bool,
    pub origin: f64,
}
impl PageAxis {
    pub fn new(
        nodes: &BTreeMap<u64, &LayoutNode>,
        styles: &ComputedStyles,
        geometry: &BTreeMap<u64, NodeGeometry>,
    ) -> Self {
        let root = nodes.values().find(|node| node.parent.is_none());
        let mode = root
            .and_then(|node| styles.get(&node.id))
            .map(|s| style_value(s, "writing-mode", "horizontal-tb"))
            .unwrap_or("horizontal-tb");
        let axes = writing_mode::Axes::new(mode, false);
        let origin = if axes.vertical && axes.x_reverse {
            root.and_then(|node| geometry.get(&node.id))
                .and_then(|g| g.box_model.as_ref())
                .map(|m| m.content_box.right())
                .unwrap_or(0.)
        } else {
            0.
        };
        Self {
            vertical: axes.vertical,
            reverse: axes.x_reverse,
            origin,
        }
    }
    pub fn extent(self, viewport: ViewportState) -> f64 {
        if self.vertical {
            viewport.width
        } else {
            viewport.height
        }
    }
    pub fn rect(self, r: CssRect) -> CssRect {
        if !self.vertical {
            r
        } else {
            CssRect {
                x: r.y,
                y: if self.reverse {
                    self.origin - r.right()
                } else {
                    r.x
                },
                width: r.height,
                height: r.width,
            }
        }
    }
    pub fn physical(self, r: CssRect) -> CssRect {
        if !self.vertical {
            r
        } else {
            CssRect {
                x: if self.reverse {
                    self.origin - r.bottom()
                } else {
                    r.y
                },
                y: r.x,
                width: r.height,
                height: r.width,
            }
        }
    }
    pub fn offset(self, top: f64, content: CssRect, scale: f64) -> (f64, f64) {
        if !self.vertical {
            (content.x, content.y - top * scale)
        } else if self.reverse {
            (content.right() - (self.origin - top) * scale, content.y)
        } else {
            (content.x - top * scale, content.y)
        }
    }
    pub fn block_property(self) -> &'static str {
        if self.vertical { "width" } else { "height" }
    }
    pub fn leading_side(self) -> &'static str {
        if !self.vertical {
            "top"
        } else if self.reverse {
            "right"
        } else {
            "left"
        }
    }
    pub fn delta(self, amount: f64) -> (f64, f64) {
        if self.vertical {
            (if self.reverse { -amount } else { amount }, 0.)
        } else {
            (0., amount)
        }
    }
}
