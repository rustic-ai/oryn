//! Per-consumer SVG media context. Resource bytes never own this state.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SvgEnvironment {
    pub width: f64,
    pub height: f64,
    pub device_pixel_ratio: f64,
    pub dark: bool,
    pub reduced_motion: bool,
}
impl Default for SvgEnvironment {
    fn default() -> Self {
        Self {
            width: 300.,
            height: 150.,
            device_pixel_ratio: 1.,
            dark: false,
            reduced_motion: false,
        }
    }
}
impl SvgEnvironment {
    pub(crate) fn for_style(
        style: &std::collections::BTreeMap<String, String>,
        rect: CssRect,
        viewport: oryn_common::v4::ViewportState,
    ) -> Self {
        let preferred = style
            .get("oryn-preferred-color-scheme")
            .is_some_and(|value| value == "dark");
        let schemes = style
            .get("color-scheme")
            .map(String::as_str)
            .unwrap_or("normal")
            .split_ascii_whitespace()
            .collect::<Vec<_>>();
        let dark = match (schemes.contains(&"light"), schemes.contains(&"dark")) {
            (true, false) => false,
            (false, true) => true,
            (true, true) => preferred,
            (false, false) => false,
        };
        Self {
            width: rect.width,
            height: rect.height,
            device_pixel_ratio: viewport.device_pixel_ratio,
            dark,
            reduced_motion: style
                .get("oryn-reduced-motion")
                .is_some_and(|value| value == "reduce"),
        }
    }
    pub(crate) fn viewport(self) -> oryn_common::v4::ViewportState {
        oryn_common::v4::ViewportState {
            width: self.width,
            height: self.height,
            device_pixel_ratio: self.device_pixel_ratio,
        }
    }
    pub(crate) fn key(self) -> [u64; 5] {
        [
            self.width.to_bits(),
            self.height.to_bits(),
            self.device_pixel_ratio.to_bits(),
            self.dark as u64,
            self.reduced_motion as u64,
        ]
    }
    pub(crate) fn current(snapshot: &PaintSnapshot) -> Self {
        CURRENT
            .with(|current| current.get())
            .unwrap_or_else(|| Self {
                width: snapshot.viewport.width,
                height: snapshot.viewport.height,
                device_pixel_ratio: snapshot.viewport.device_pixel_ratio,
                ..Self::default()
            })
    }
    pub(crate) fn enter(self) -> Scope {
        Scope(CURRENT.with(|current| current.replace(Some(self))))
    }
}
// Lexical propagation to nested data SVG/foreignObject and semantic callbacks;
// RAII restores the outer consumer on failure or reentrant parsing.
thread_local! {static CURRENT:std::cell::Cell<Option<SvgEnvironment>>=const{std::cell::Cell::new(None)};}
pub(crate) struct Scope(Option<SvgEnvironment>);
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.set(self.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consumer_scheme_negotiation_and_reentrant_context_restore() {
        let snapshot = crate::tests::empty_snapshot();
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 20.,
            height: 10.,
        };
        let mut style = std::collections::BTreeMap::from([
            ("oryn-preferred-color-scheme".into(), "dark".into()),
            ("oryn-reduced-motion".into(), "reduce".into()),
            ("color-scheme".into(), "light dark".into()),
        ]);
        let preferred = SvgEnvironment::for_style(&style, rect, snapshot.viewport);
        assert!(preferred.dark && preferred.reduced_motion);
        style.insert("color-scheme".into(), "only light".into());
        let light = SvgEnvironment::for_style(&style, rect, snapshot.viewport);
        assert!(!light.dark);
        style.insert("color-scheme".into(), "light dark".into());
        assert!(SvgEnvironment::for_style(&style, rect, snapshot.viewport).dark);
        let original = SvgEnvironment::current(&snapshot);
        {
            let _outer = preferred.enter();
            {
                let _inner = light.enter();
                assert!(!SvgEnvironment::current(&snapshot).dark);
            }
            assert!(SvgEnvironment::current(&snapshot).dark);
        }
        assert_eq!(SvgEnvironment::current(&snapshot), original);
    }
    #[test]
    fn media_viewport_is_css_image_size_not_viewbox_or_output_raster() {
        let snapshot = crate::tests::empty_snapshot();
        let source=oryn_common::v4::PaintImageResource{svg_document_snapshot:false,content_type:None,url_aliases:Vec::new(),url:"https://example.test/media.svg".into(),media_type:"image/svg+xml".into(),bytes:br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 1000 1000"><style>rect {fill:red} @media (min-width: 30px) {rect {fill:lime}} @media (prefers-color-scheme:dark) {rect {fill:blue}} @media (prefers-reduced-motion:reduce) {rect {fill:white}}</style><rect width="1000" height="1000"/></svg>"#.to_vec()};
        for (width, dark, reduced, expected) in [
            (20., false, false, [255, 0, 0, 255]),
            (40., false, false, [0, 255, 0, 255]),
            (20., true, false, [0, 0, 255, 255]),
            (20., false, true, [255, 255, 255, 255]),
        ] {
            let environment = SvgEnvironment {
                width,
                height: 20.,
                dark,
                reduced_motion: reduced,
                device_pixel_ratio: 2.,
            };
            let tree = crate::svg::parse_with_environment(&source, &snapshot, environment).unwrap();
            for resolution in [10, 80] {
                let rgba = crate::svg::raster(&tree, resolution, resolution).unwrap();
                assert_eq!(rgba.get_pixel(resolution / 2, resolution / 2).0, expected);
            }
        }
    }
}

#[cfg(test)]
mod nested_media_tests {
    use super::*;
    use base64::Engine;
    #[test]
    fn nested_same_data_svg_uses_each_image_viewport_and_used_scheme() {
        let nested=br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><style>rect{fill:red}@media(min-width:30px){rect{fill:lime}}@media(prefers-color-scheme:dark){rect{fill:blue}}</style><rect width="10" height="10"/></svg>"#;
        let href = format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(nested)
        );
        let outer = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="20"><image href="{href}" width="20" height="20" preserveAspectRatio="none"/><image href="{href}" x="30" width="40" height="20" preserveAspectRatio="none"/><image href="{href}" x="80" width="20" height="20" style="color-scheme:dark"/></svg>"#
        );
        let source = oryn_common::v4::PaintImageResource {
            svg_document_snapshot: false,
            content_type: None,
            url_aliases: Vec::new(),
            url: "https://example.test/nested-media.svg".into(),
            media_type: "image/svg+xml".into(),
            bytes: outer.into_bytes(),
        };
        let snapshot = crate::tests::empty_snapshot();
        let tree = crate::svg::parse_with_environment(
            &source,
            &snapshot,
            SvgEnvironment {
                width: 100.,
                height: 20.,
                ..SvgEnvironment::default()
            },
        )
        .unwrap();
        let pixels = crate::svg::raster(&tree, 100, 20).unwrap();
        assert_eq!(pixels.get_pixel(5, 10).0, [255, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(40, 10).0, [0, 255, 0, 255]);
        assert_eq!(pixels.get_pixel(90, 10).0, [0, 0, 255, 255]);
    }
}
