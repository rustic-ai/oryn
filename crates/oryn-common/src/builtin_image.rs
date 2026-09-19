//! Typed UA images have no URL lookup or arbitrary SVG-source entry point.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionIconKind {
    Location,
    PreciseLocation,
    UserMedia,
    Install,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum BuiltinImage {
    PermissionIcon {
        kind: PermissionIconKind,
        fill: [u8; 4],
        stroke: [u8; 4],
        stroke_width: f64,
    },
}
impl BuiltinImage {
    pub fn svg(&self) -> Result<Vec<u8>, String> {
        match self {
            Self::PermissionIcon {
                kind,
                fill,
                stroke,
                stroke_width,
            } => {
                if !stroke_width.is_finite() || !(0.0..=24.).contains(stroke_width) {
                    return Err("invalid native permission icon stroke".into());
                }
                let color = |rgba: &[u8; 4]| format!("rgb({},{},{})", rgba[0], rgba[1], rgba[2]);
                let shape = match kind {
                    PermissionIconKind::Install => {
                        "<path d='M10 2h4v10h4l-6 6-6-6h4zM3 19h18v3H3z'/>"
                    }
                    PermissionIconKind::Location => {
                        "<path d='M12 2a7 7 0 0 0-7 7c0 5 7 13 7 13s7-8 7-13a7 7 0 0 0-7-7z'/><circle cx='12' cy='9' r='2' fill='white' stroke='none'/>"
                    }
                    PermissionIconKind::PreciseLocation => {
                        "<circle cx='12' cy='12' r='7'/><path d='M10 0h4v5h-4zM10 19h4v5h-4zM0 10h5v4H0zM19 10h5v4h-5z'/><circle cx='12' cy='12' r='3' fill='white' stroke='none'/>"
                    }
                    PermissionIconKind::UserMedia => {
                        "<rect x='2' y='5' width='15' height='14' rx='2'/><path d='M17 9l5-3v12l-5-3z'/><circle cx='9' cy='12' r='3' fill='white' stroke='none'/>"
                    }
                };
                let svg = format!(
                    "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 24 24'><g fill='{}' fill-opacity='{}' stroke='{}' stroke-opacity='{}' stroke-width='{}'>{shape}</g></svg>",
                    color(fill),
                    f64::from(fill[3]) / 255.,
                    color(stroke),
                    f64::from(stroke[3]) / 255.,
                    stroke_width
                );
                if svg.len() > 2048 {
                    return Err("native permission icon source bound".into());
                }
                Ok(svg.into_bytes())
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_icon_has_no_url_or_untrusted_svg_entry_point() {
        let icon = BuiltinImage::PermissionIcon {
            kind: PermissionIconKind::Location,
            fill: [255, 0, 0, 255],
            stroke: [0, 0, 0, 0],
            stroke_width: 1.,
        };
        let svg = String::from_utf8(icon.svg().unwrap()).unwrap();
        assert!(!svg.contains("href="));
        assert!(!svg.contains("<script"));
        assert!(
            serde_json::from_str::<BuiltinImage>(r#"{"url":"oryn-native:permission-icon"}"#)
                .is_err()
        );
        assert!(
            BuiltinImage::PermissionIcon {
                kind: PermissionIconKind::Location,
                fill: [0; 4],
                stroke: [0; 4],
                stroke_width: f64::NAN
            }
            .svg()
            .is_err()
        );
    }
}
