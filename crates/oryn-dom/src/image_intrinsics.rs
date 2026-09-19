//! Intrinsic dimensions are separate from the default concrete object size.
use crate::{Node, NodeKind};
use serde::{Deserialize, Serialize};

/// Finite nonnegative lengths encoded by bits preserve Node's exact Eq identity.
/// Deserialized nonfinite/negative values are treated as absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageIntrinsics {
    width: Option<u64>,
    height: Option<u64>,
    ratio: Option<u64>,
}
impl ImageIntrinsics {
    pub fn new(width: Option<f64>, height: Option<f64>, ratio: Option<f64>) -> Self {
        let valid = |value: Option<f64>| {
            value
                .filter(|v| v.is_finite() && *v >= 0.)
                .map(f64::to_bits)
        };
        Self {
            width: valid(width),
            height: valid(height),
            ratio: valid(ratio.filter(|v| *v > 0.)),
        }
    }
    fn value(value: Option<u64>) -> Option<f64> {
        value
            .map(f64::from_bits)
            .filter(|v| v.is_finite() && *v >= 0.)
    }
    /// Computed SVG geometry overrides presentation dimensions without turning
    /// a previous concrete size into a natural aspect ratio.
    pub fn with_dimensions(self, width: Option<f64>, height: Option<f64>) -> Self {
        Self::new(width, height, Self::value(self.ratio))
    }
    pub fn width(self) -> Option<f64> {
        Self::value(self.width)
    }
    pub fn height(self) -> Option<f64> {
        Self::value(self.height)
    }
    pub fn ratio(self) -> Option<f64> {
        match (self.width(), self.height()) {
            (Some(w), Some(h)) => {
                let ratio = w / h;
                (ratio.is_finite() && ratio > 0.).then_some(ratio)
            }
            _ => Self::value(self.ratio).filter(|v| *v > 0.),
        }
    }
    pub fn default_size(self) -> (u32, u32) {
        let (w, h) = match (self.width(), self.height(), self.ratio()) {
            (Some(w), Some(h), _) => (w, h),
            (Some(w), None, Some(r)) => (w, w / r),
            (None, Some(h), Some(r)) => (h * r, h),
            (None, None, Some(r)) if r > 2. => (300., 300. / r),
            (None, None, Some(r)) => (150. * r, 150.),
            (w, h, None) => (w.unwrap_or(300.), h.unwrap_or(150.)),
        };
        (w as u32, h as u32)
    }
    pub fn from_svg(root: &Node) -> Option<Self> {
        if !matches!(&root.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/2000/svg"&&name.local.as_ref()=="svg")
        {
            return None;
        }
        let length = |name| {
            root.attributes
                .get_null(name)
                .and_then(|value| absolute_length(value))
        };
        let ratio = root.attributes.get_null("viewBox").and_then(|value| {
            let values = value
                .split(|c: char| c.is_ascii_whitespace() || c == ',')
                .filter(|s| !s.is_empty())
                .map(str::parse::<f64>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            (values.len() == 4
                && values.iter().all(|v| v.is_finite())
                && values[2] > 0.
                && values[3] > 0.)
                .then(|| values[2] / values[3])
        });
        Some(Self::new(length("width"), length("height"), ratio))
    }
}
fn absolute_length(value: &str) -> Option<f64> {
    let lower = value.trim().to_ascii_lowercase();
    let value = lower.as_str();
    let (number, scale) = [
        ("px", 1.),
        ("in", 96.),
        ("cm", 96. / 2.54),
        ("mm", 96. / 25.4),
        ("q", 96. / 101.6),
        ("pt", 96. / 72.),
        ("pc", 16.),
    ]
    .into_iter()
    .find_map(|(unit, scale)| value.strip_suffix(unit).map(|value| (value, scale)))
    .unwrap_or((value, 1.));
    let value = number.trim().parse::<f64>().ok()? * scale;
    (value.is_finite() && value >= 0.).then_some(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concrete_default_size_never_fabricates_natural_dimensions_or_ratio() {
        let blank = ImageIntrinsics::new(None, None, None);
        assert_eq!(blank.default_size(), (300, 150));
        assert_eq!(blank.ratio(), None);
        let width = ImageIntrinsics::new(Some(80.), None, None);
        assert_eq!(width.default_size(), (80, 150));
        assert_eq!(width.height(), None);
        assert_eq!(width.ratio(), None);
        let ratio = ImageIntrinsics::new(None, None, Some(1.));
        assert_eq!(ratio.default_size(), (150, 150));
        assert_eq!(ratio.width(), None);
        assert_eq!(
            ImageIntrinsics::new(Some(80.), None, Some(2.)).default_size(),
            (80, 40)
        );
        assert_eq!(
            ImageIntrinsics::new(Some(f64::NAN), None, Some(f64::INFINITY)).default_size(),
            (300, 150)
        );
    }
}
