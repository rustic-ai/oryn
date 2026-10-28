//! Native CSS media query parser and evaluator shared by layout and page algorithms.

/// Inputs for source size.
pub struct SourceSizeInputs<'a> {
    pub sizes: &'a str,
    pub base_url: &'a str,
    pub quirks: bool,
    pub viewport: ViewportState,
    pub color_scheme: ColorScheme,
    pub reduced_motion: bool,
    pub print_media: bool,
    pub auto_width: Option<f64>,
}
use super::{BaseDocument, BlitzColorScheme, BlitzViewport, ColorScheme, DocumentConfig};
use oryn_common::v4::ViewportState;

pub fn matches(
    query: &str,
    base_url: &str,
    quirks: bool,
    viewport: ViewportState,
    color_scheme: ColorScheme,
    reduced_motion: bool,
    print_media: bool,
) -> Result<bool, String> {
    use style::{
        context::QuirksMode,
        media_queries::{MediaList, MediaType},
        parser::ParserContext,
        stylesheets::{CssRuleType, CustomMediaEvaluator, Origin, UrlExtraData},
    };
    let url = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let quirks = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let mut context = ParserContext::new(
        Origin::Author,
        &url,
        Some(CssRuleType::Media),
        style_traits::ParsingMode::DEFAULT,
        quirks,
        Default::default(),
        None,
        None,
        Default::default(),
    );
    let mut text = cssparser::ParserInput::new(query);
    let list = MediaList::parse(&mut context, &mut cssparser::Parser::new(&mut text));
    let mut document = BaseDocument::new(DocumentConfig {
        media_type: Some(if print_media {
            MediaType::print()
        } else {
            MediaType::screen()
        }),
        viewport: Some(BlitzViewport::new(
            (viewport.width * viewport.device_pixel_ratio).clamp(1., f64::from(u32::MAX)) as u32,
            (viewport.height * viewport.device_pixel_ratio).clamp(1., f64::from(u32::MAX)) as u32,
            viewport.device_pixel_ratio as f32,
            match color_scheme {
                ColorScheme::Dark => BlitzColorScheme::Dark,
                ColorScheme::Light => BlitzColorScheme::Light,
            },
        )),
        base_url: Some(base_url.into()),
        style_threading: blitz_dom::StyleThreading::Sequential,
        ..Default::default()
    });
    document.set_prefers_reduced_motion(reduced_motion);
    document.set_scripting_enabled(true);
    Ok(list.evaluate(
        document.stylist_device(),
        quirks,
        &mut CustomMediaEvaluator::none(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scripting_media_tracks_device_state_and_survives_viewport_changes() {
        use style::{
            context::QuirksMode,
            media_queries::MediaList,
            parser::ParserContext,
            stylesheets::{CssRuleType, CustomMediaEvaluator, Origin, UrlExtraData},
        };
        let url = UrlExtraData::from(url::Url::parse("https://example.test/").unwrap());
        let parse = |query: &str| {
            let mut context = ParserContext::new(
                Origin::Author,
                &url,
                Some(CssRuleType::Media),
                style_traits::ParsingMode::DEFAULT,
                QuirksMode::NoQuirks,
                Default::default(),
                None,
                None,
                Default::default(),
            );
            let mut text = cssparser::ParserInput::new(query);
            MediaList::parse(&mut context, &mut cssparser::Parser::new(&mut text))
        };
        let queries = [
            parse("(scripting)"),
            parse("(scripting: enabled)"),
            parse("(scripting: none)"),
            parse("(scripting: initial-only)"),
        ];
        let mut document = BaseDocument::new(DocumentConfig::default());
        for enabled in [false, true, false] {
            document.set_scripting_enabled(enabled);
            document.set_css_viewport_size(400., 300.);
            assert_eq!(document.stylist_device().scripting_enabled(), enabled);
            for (query, expected) in queries.iter().zip([enabled, enabled, !enabled, false]) {
                assert_eq!(
                    query.evaluate(
                        document.stylist_device(),
                        QuirksMode::NoQuirks,
                        &mut CustomMediaEvaluator::none()
                    ),
                    expected
                );
            }
        }
    }
    #[test]
    fn native_media_lists_ranges_negation_invalid_and_preferences() {
        let viewport = ViewportState {
            width: 800.,
            height: 600.,
            device_pixel_ratio: 2.,
        };
        let evaluate = |query| {
            matches(
                query,
                "https://media.test/",
                false,
                viewport,
                ColorScheme::Dark,
                true,
                false,
            )
            .unwrap()
        };
        for query in [
            "",
            "screen and (600px < width < 900px)",
            "print, (orientation: landscape)",
            "(resolution: 2dppx)",
            "(prefers-color-scheme: dark) and (prefers-reduced-motion: reduce)",
            "not print",
            "(scripting)",
            "(scripting: enabled)",
        ] {
            assert!(evaluate(query), "{query}");
        }
        for query in [
            "print",
            "(scripting: none)",
            "(scripting: initial-only)",
            "(width > 900px)",
            "(prefers-reduced-motion: no-preference)",
            "(unknown-feature)",
            "screen and (",
        ] {
            assert!(!evaluate(query), "{query}");
        }
    }
}

/// The HTML sizes algorithm uses the CSS parser and media-query evaluation device,
/// including calc(), viewport units, font-relative lengths and invalid-item recovery.
pub fn source_size(params: SourceSizeInputs<'_>) -> Result<f64, String> {
    let SourceSizeInputs {
        sizes,
        base_url,
        quirks,
        viewport,
        color_scheme,
        reduced_motion,
        print_media,
        auto_width,
    } = params;

    use style::{
        context::QuirksMode,
        media_queries::MediaType,
        parser::ParserContext,
        stylesheets::{CssRuleType, Origin, UrlExtraData},
    };
    let url = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let quirks = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let context = ParserContext::new(
        Origin::Author,
        &url,
        Some(CssRuleType::Media),
        style_traits::ParsingMode::DEFAULT,
        quirks,
        Default::default(),
        None,
        None,
        Default::default(),
    );
    let mut input = sizes.trim();
    if let Some(rest) = input.get(4..).filter(|rest| {
        input
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("auto"))
            && (rest.is_empty() || rest.starts_with(','))
    }) {
        if let Some(width) = auto_width.filter(|width| width.is_finite() && *width >= 0.) {
            return Ok(width);
        }
        input = rest
            .trim_start()
            .strip_prefix(',')
            .unwrap_or(rest)
            .trim_start();
    }
    let mut text = cssparser::ParserInput::new(input);
    let list = style::values::specified::source_size_list::SourceSizeList::parse(
        &context,
        &mut cssparser::Parser::new(&mut text),
    );
    let mut document = BaseDocument::new(DocumentConfig {
        media_type: Some(if print_media {
            MediaType::print()
        } else {
            MediaType::screen()
        }),
        viewport: Some(BlitzViewport::new(
            (viewport.width * viewport.device_pixel_ratio).clamp(1., f64::from(u32::MAX)) as u32,
            (viewport.height * viewport.device_pixel_ratio).clamp(1., f64::from(u32::MAX)) as u32,
            viewport.device_pixel_ratio as f32,
            match color_scheme {
                ColorScheme::Dark => BlitzColorScheme::Dark,
                ColorScheme::Light => BlitzColorScheme::Light,
            },
        )),
        base_url: Some(base_url.into()),
        style_threading: blitz_dom::StyleThreading::Sequential,
        ..Default::default()
    });
    document.set_prefers_reduced_motion(reduced_motion);
    document.set_scripting_enabled(true);
    Ok(list.evaluate(document.stylist_device(), quirks).to_f64_px())
}

#[cfg(test)]
mod source_size_tests {
    use super::*;
    #[test]
    fn responsive_sizes_use_css_math_media_and_initial_font() {
        let evaluate = |sizes, auto| {
            source_size(SourceSizeInputs {
                sizes,
                base_url: "https://images.test/",
                quirks: false,
                viewport: ViewportState {
                    width: 800.,
                    height: 600.,
                    device_pixel_ratio: 2.,
                },
                color_scheme: ColorScheme::Light,
                reduced_motion: false,
                print_media: false,
                auto_width: auto,
            })
            .unwrap()
        };
        assert_eq!(
            evaluate("(width < 600px) 20px, calc(50vw - 10px)", None),
            390.
        );
        assert_eq!(evaluate("broken, -1px, 50%, 10em", None), 160.);
        assert_eq!(evaluate("AUTO, 50vw", Some(12.)), 12.);
        assert_eq!(evaluate("auto, 50vw", None), 400.);
        assert_eq!(evaluate("", None), 800.);
    }
}
