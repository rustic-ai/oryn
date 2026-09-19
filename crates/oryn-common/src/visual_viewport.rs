//! Shared native visual viewport transform for input, capture and painting.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    /// Physical content viewport excluding classic gutters; None is overlay mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollport: Option<crate::v4::CssRect>,
    pub layout_width: f64,
    pub layout_height: f64,
    pub scale: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}
impl Default for Geometry {
    fn default() -> Self {
        Self {
            scrollport: None,
            layout_width: 0.,
            layout_height: 0.,
            scale: 1.,
            offset_x: 0.,
            offset_y: 0.,
        }
    }
}
impl Geometry {
    /// Validate serialized/native boundary data before consuming its transform.
    pub fn is_valid(self) -> bool {
        let port = self.content_rect();
        [
            self.layout_width,
            self.layout_height,
            self.scale,
            self.offset_x,
            self.offset_y,
        ]
        .iter()
        .all(|value| value.is_finite())
            && [
                port.x,
                port.y,
                port.width,
                port.height,
                port.right(),
                port.bottom(),
            ]
            .iter()
            .all(|value| value.is_finite())
            && port.x >= 0.
            && port.y >= 0.
            && port.width >= 0.
            && port.height >= 0.
            && port.right() <= self.layout_width
            && port.bottom() <= self.layout_height
            && self.layout_width >= 0.
            && self.layout_height >= 0.
            && self.scale > 0.
            && self.width().is_finite()
            && self.height().is_finite()
            && self.offset_x >= 0.
            && self.offset_y >= 0.
            && self.offset_x <= (port.width - self.width()).max(0.)
            && self.offset_y <= (port.height - self.height()).max(0.)
    }

    pub fn content_rect(self) -> crate::v4::CssRect {
        self.scrollport.unwrap_or(crate::v4::CssRect {
            x: 0.,
            y: 0.,
            width: self.layout_width,
            height: self.layout_height,
        })
    }
    pub fn width(self) -> f64 {
        self.content_rect().width / self.scale
    }
    pub fn height(self) -> f64 {
        self.content_rect().height / self.scale
    }
    pub fn resize(&mut self, width: f64, height: f64) -> bool {
        if ![width, height]
            .iter()
            .all(|value| value.is_finite() && *value >= 0.)
        {
            return false;
        }
        let old = *self;
        self.layout_width = width;
        self.layout_height = height;
        self.scrollport = None;
        self.clamp();
        old != *self
    }
    pub fn set_scrollport(&mut self, port: Option<crate::v4::CssRect>) -> bool {
        if port.is_some_and(|port| {
            ![
                port.x,
                port.y,
                port.width,
                port.height,
                port.right(),
                port.bottom(),
            ]
            .iter()
            .all(|value| value.is_finite())
                || port.x < 0.
                || port.y < 0.
                || port.width < 0.
                || port.height < 0.
                || port.right() > self.layout_width
                || port.bottom() > self.layout_height
        }) {
            return false;
        }
        let old = *self;
        self.scrollport = port;
        self.clamp();
        old != *self
    }
    /// Clamp offsets after a validated native scale or viewport extent update.
    pub fn clamp(&mut self) {
        self.offset_x = self
            .offset_x
            .clamp(0., (self.content_rect().width - self.width()).max(0.));
        self.offset_y = self
            .offset_y
            .clamp(0., (self.content_rect().height - self.height()).max(0.));
    }
    pub fn visual_to_layout(self, x: f64, y: f64) -> Option<(f64, f64)> {
        if !self.is_valid() {
            return None;
        }
        let port = self.content_rect();
        let point = (
            port.x + self.offset_x + (x - port.x) / self.scale,
            port.y + self.offset_y + (y - port.y) / self.scale,
        );
        (point.0.is_finite() && point.1.is_finite()).then_some(point)
    }
    pub fn layout_to_visual(self, x: f64, y: f64) -> Option<(f64, f64)> {
        if !self.is_valid() {
            return None;
        }
        let port = self.content_rect();
        let point = (
            port.x + (x - port.x - self.offset_x) * self.scale,
            port.y + (y - port.y - self.offset_y) * self.scale,
        );
        (point.0.is_finite() && point.1.is_finite()).then_some(point)
    }
    pub fn pan(&mut self, dx: f64, dy: f64) -> Option<(f64, f64)> {
        if !self.is_valid() || !dx.is_finite() || !dy.is_finite() {
            return None;
        }
        let old = (self.offset_x, self.offset_y);
        self.offset_x += dx / self.scale;
        self.offset_y += dy / self.scale;
        self.clamp();
        // Residual remains in visual CSS pixels for outer scrolling consumers.
        Some((
            dx - (self.offset_x - old.0) * self.scale,
            dy - (self.offset_y - old.1) * self.scale,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classic_gutters_bound_camera_and_preserve_content_origin() {
        let mut viewport = Geometry {
            scrollport: None,
            layout_width: 200.,
            layout_height: 100.,
            scale: 2.,
            offset_x: 100.,
            offset_y: 50.,
        };
        let port = crate::v4::CssRect {
            x: 16.,
            y: 0.,
            width: 184.,
            height: 84.,
        };
        assert!(viewport.set_scrollport(Some(port)));
        assert!(viewport.is_valid());
        assert_eq!(
            (
                viewport.width(),
                viewport.height(),
                viewport.offset_x,
                viewport.offset_y
            ),
            (92., 42., 92., 42.)
        );
        assert_eq!(viewport.visual_to_layout(port.x, port.y), Some((108., 42.)));
        assert_eq!(viewport.layout_to_visual(200., 84.), Some((200., 84.)));
        assert_eq!(viewport.pan(100., 100.), Some((100., 100.)));
        let before = viewport;
        assert!(!viewport.set_scrollport(Some(crate::v4::CssRect {
            width: 201.,
            ..port
        })));
        assert_eq!(viewport, before);
    }
    #[test]
    fn shared_viewport_rejects_invalid_serialized_geometry() {
        let valid = Geometry {
            scrollport: None,
            layout_width: 400.,
            layout_height: 300.,
            scale: 2.,
            offset_x: 100.,
            offset_y: 75.,
        };
        assert!(valid.is_valid());
        for invalid in [
            Geometry { scale: 0., ..valid },
            Geometry {
                scale: f64::NAN,
                ..valid
            },
            Geometry {
                offset_x: 201.,
                ..valid
            },
            Geometry {
                layout_height: -1.,
                ..valid
            },
            Geometry {
                offset_y: f64::INFINITY,
                ..valid
            },
        ] {
            assert!(!invalid.is_valid());
            assert_eq!(invalid.visual_to_layout(0., 0.), None);
            assert_eq!(invalid.layout_to_visual(0., 0.), None);
        }
    }
    #[test]
    fn input_and_paint_share_inverse_viewport_mapping() {
        let viewport = Geometry {
            scrollport: None,
            layout_width: 400.,
            layout_height: 300.,
            scale: 2.5,
            offset_x: 40.,
            offset_y: 30.,
        };
        for point in [(0., 0.), (37.5, 82.5), (400., 300.), (-20., -10.)] {
            let layout = viewport.visual_to_layout(point.0, point.1).unwrap();
            let painted = viewport.layout_to_visual(layout.0, layout.1).unwrap();
            assert!((point.0 - painted.0).abs() < 1e-9 && (point.1 - painted.1).abs() < 1e-9);
        }
    }
}
