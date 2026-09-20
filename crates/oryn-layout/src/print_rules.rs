//! Page-context rules use Stylo's parser, selector specificity and media query
//! evaluator. Element selector cascade is deliberately not reused for @page.
use super::*;
use style::{
    context::QuirksMode,
    properties::PropertyDeclarationBlock,
    shared_lock::SharedRwLockReadGuard,
    stylesheets::{CssRule, PagePseudoClassFlags, PageSelectors},
};
type PageDeclarations = BTreeMap<String, (String, bool)>;
#[derive(Clone)]
pub(super) struct Rule {
    selectors: PageSelectors,
    declarations: PageDeclarations,
    pub margins: Vec<(String, PageDeclarations)>,
}
fn declarations(block: &PropertyDeclarationBlock) -> PageDeclarations {
    block
        .declaration_importance_iter()
        .map(|(value, importance)| {
            let mut css = String::new();
            value
                .to_css(&mut css)
                .expect("writing a CSS value to String cannot fail");
            (
                value.id().name().into_owned(),
                (css, importance.important()),
            )
        })
        .collect()
}
fn collect(
    rules: &[CssRule],
    guard: &SharedRwLockReadGuard,
    device: &style::device::Device,
    out: &mut Vec<Rule>,
) {
    use style::stylesheets::CustomMediaEvaluator;
    for rule in rules {
        match rule {
            CssRule::Page(page) => {
                let page = page.read_with(guard);
                let margins = page
                    .rules
                    .read_with(guard)
                    .0
                    .iter()
                    .filter_map(|child| {
                        let CssRule::Margin(margin) = child else {
                            return None;
                        };
                        Some((
                            format!("{:?}", margin.rule_type),
                            declarations(margin.block.read_with(guard)),
                        ))
                    })
                    .collect();
                out.push(Rule {
                    selectors: page.selectors.clone(),
                    declarations: declarations(page.block.read_with(guard)),
                    margins,
                });
            }
            CssRule::Media(media) => {
                if media.media_queries.read_with(guard).evaluate(
                    device,
                    QuirksMode::NoQuirks,
                    &mut CustomMediaEvaluator::none(),
                ) {
                    collect(rule.children(guard), guard, device, out);
                }
            }
            CssRule::Supports(supports) if supports.enabled => {
                collect(rule.children(guard), guard, device, out)
            }
            CssRule::LayerBlock(_) => collect(rule.children(guard), guard, device, out),
            _ => {}
        }
    }
}
pub(super) fn read(input: &LayoutInput) -> Vec<Rule> {
    use style::{
        media_queries::{MediaList, MediaType},
        servo_arc::Arc,
        shared_lock::SharedRwLock,
        stylesheets::{AllowImportRules, Origin, Stylesheet, StylesheetInDocument, UrlExtraData},
    };
    let viewport = BlitzViewport::new(
        input.viewport.width.max(1.) as u32,
        input.viewport.height.max(1.) as u32,
        1.,
        match input.color_scheme {
            ColorScheme::Light => BlitzColorScheme::Light,
            ColorScheme::Dark => BlitzColorScheme::Dark,
        },
    );
    let mut document = BaseDocument::new(DocumentConfig {
        viewport: Some(viewport),
        media_type: Some(MediaType::print()),
        style_threading: blitz_dom::StyleThreading::Sequential,
        ..Default::default()
    });
    let device = document.stylist_device();
    let mut result = Vec::new();
    for (index, source) in input.stylesheets.iter().enumerate() {
        // Shadow-scoped sheets do not establish the document's page context.
        if input.shadow_trees.stylesheet_scopes.contains_key(&index) {
            continue;
        }
        let source = if let Some(media) = input.stylesheet_media.get(&index) {
            format!("@media {media} {{{source}}}")
        } else {
            source.clone()
        };
        let lock = SharedRwLock::new();
        let base = input
            .stylesheet_base_urls
            .get(&index)
            .and_then(|base| url::Url::parse(base).ok())
            .unwrap_or_else(|| url::Url::parse("https://oryn.invalid/").unwrap());
        let sheet = Stylesheet::from_str(
            &source,
            UrlExtraData::from(base),
            Origin::Author,
            Arc::new(lock.wrap(MediaList::empty())),
            lock.clone(),
            None,
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::No,
        );
        let guard = sheet.shared_lock.read();
        collect(
            &sheet.contents(&guard).rules.read_with(&guard).0,
            &guard,
            device,
            &mut result,
        );
    }
    result
}
fn specificity(rule: &Rule, name: &str, index: usize, blank: bool) -> Option<u32> {
    let mut flags = if index.is_multiple_of(2) {
        PagePseudoClassFlags::RIGHT
    } else {
        PagePseudoClassFlags::LEFT
    };
    if index == 0 {
        flags |= PagePseudoClassFlags::FIRST;
    }
    if blank {
        flags |= PagePseudoClassFlags::BLANK;
    }
    if rule.selectors.is_empty() {
        return Some(0);
    }
    rule.selectors
        .as_slice()
        .iter()
        .filter(|s| s.name.0.is_empty() || s.name.0.as_ref() == name)
        .filter_map(|s| s.match_specificity(flags))
        .max()
}
pub(super) fn cascade(
    rules: &[Rule],
    name: &str,
    index: usize,
    blank: bool,
) -> BTreeMap<String, String> {
    let mut result = BTreeMap::<String, ((bool, u32, usize), String)>::new();
    for (order, rule) in rules.iter().enumerate() {
        let Some(specificity) = specificity(rule, name, index, blank) else {
            continue;
        };
        for (property, (value, important)) in &rule.declarations {
            let rank = (*important, specificity, order);
            if result.get(property).is_none_or(|(old, _)| rank >= *old) {
                result.insert(property.clone(), (rank, value.clone()));
            }
        }
    }
    result
        .into_iter()
        .map(|(name, (_, value))| (name, value))
        .collect()
}
pub(super) fn dimensions(
    base: PrintDimensions,
    style: &BTreeMap<String, String>,
) -> Result<PrintDimensions, PrintError> {
    let mut width = base.width;
    let mut height = base.height;
    if let Some(size) = style.get("size") {
        let parts = size.split_whitespace().collect::<Vec<_>>();
        let paper = parts.iter().find_map(|s| match *s {
            "a5" => Some((148., 210.)),
            "a4" => Some((210., 297.)),
            "a3" => Some((297., 420.)),
            "b5" => Some((176., 250.)),
            "b4" => Some((250., 353.)),
            "letter" => Some((215.9, 279.4)),
            "legal" => Some((215.9, 355.6)),
            "ledger" => Some((279.4, 431.8)),
            _ => None,
        });
        if let Some((w, h)) = paper {
            width = w * 96. / 25.4;
            height = h * 96. / 25.4;
        } else if parts
            .first()
            .is_some_and(|p| !matches!(*p, "auto" | "portrait" | "landscape"))
        {
            width = length(parts[0], base.width, 16., ViewportState::default());
            height = length(
                parts.get(1).copied().unwrap_or(parts[0]),
                base.height,
                16.,
                ViewportState::default(),
            );
        }
        if parts.contains(&"landscape") && width < height
            || parts.contains(&"portrait") && width > height
        {
            std::mem::swap(&mut width, &mut height);
        }
    }
    let viewport = ViewportState {
        width,
        height,
        ..Default::default()
    };
    let font_size = length(style_value(style, "font-size", "16px"), 16., 16., viewport);
    let margin = |name: &str, fallback: f64, basis: f64| {
        style
            .get(name)
            .map_or(fallback, |value| length(value, basis, font_size, viewport))
    };
    let left = margin("margin-left", base.content.x, width);
    let right = margin("margin-right", base.width - base.content.right(), width);
    let top = margin("margin-top", base.content.y, height);
    let bottom = margin("margin-bottom", base.height - base.content.bottom(), height);
    if let Some(content_width) = optional_length(
        style_value(style, "width", "auto"),
        width,
        font_size,
        viewport,
    ) {
        width = content_width + left + right;
    }
    if let Some(content_height) = optional_length(
        style_value(style, "height", "auto"),
        height,
        font_size,
        viewport,
    ) {
        height = content_height + top + bottom;
    }
    let content = CssRect {
        x: left,
        y: top,
        width: width - left - right,
        height: height - top - bottom,
    };
    if [
        width,
        height,
        left,
        right,
        top,
        bottom,
        content.width,
        content.height,
    ]
    .iter()
    .any(|v| !v.is_finite())
        || width < 1.
        || height < 1.
        || width > 200. * PX_PER_CM
        || height > 200. * PX_PER_CM
        || content.width < 1.
        || content.height < 1.
        || [left, right, top, bottom].iter().any(|v| *v < 0.)
    {
        return Err(PrintError::Invalid(
            "@page dimensions exceed print resource bounds".into(),
        ));
    }
    Ok(PrintDimensions {
        width,
        height,
        content,
    })
}

pub(super) fn margin_styles(
    rules: &[Rule],
    name: &str,
    index: usize,
    blank: bool,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut result = BTreeMap::<String, BTreeMap<String, ((bool, u32, usize), String)>>::new();
    for (order, rule) in rules.iter().enumerate() {
        let Some(specificity) = specificity(rule, name, index, blank) else {
            continue;
        };
        for (kind, properties) in &rule.margins {
            let target = result.entry(kind.clone()).or_default();
            for (property, (value, important)) in properties {
                let rank = (*important, specificity, order);
                if target.get(property).is_none_or(|(old, _)| rank >= *old) {
                    target.insert(property.clone(), (rank, value.clone()));
                }
            }
        }
    }
    let page = cascade(rules, name, index, blank);
    result
        .into_iter()
        .map(|(kind, properties)| {
            let mut inherited = page
                .iter()
                .filter(|(key, _)| {
                    key.starts_with("font-")
                        || matches!(
                            key.as_str(),
                            "color"
                                | "line-height"
                                | "white-space"
                                | "white-space-collapse"
                                | "text-wrap-mode"
                                | "writing-mode"
                                | "text-orientation"
                                | "direction"
                                | "letter-spacing"
                                | "word-spacing"
                        )
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<BTreeMap<_, _>>();
            inherited.extend(properties.into_iter().map(|(key, (_, value))| (key, value)));
            (kind, inherited)
        })
        .collect()
}
/// CSS string tokens are decoded by cssparser, including escapes. Page counters
/// use physical page numbers before WebDriver's page-range selection.
pub(super) fn margin_content(value: &str, page: usize, total: usize) -> Option<String> {
    use cssparser::{Parser, ParserInput, Token};
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    let mut result = String::new();
    while !parser.is_exhausted() {
        match parser.next().ok()?.clone() {
            Token::QuotedString(text) => result.push_str(&text),
            Token::Ident(value)
                if value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("normal") =>
            {
                return None;
            }
            Token::Function(name) if name.eq_ignore_ascii_case("counter") => {
                let number = parser
                    .parse_nested_block(|p| {
                        let name = p.expect_ident_cloned()?;
                        let number = match name.as_ref() {
                            "page" => page,
                            "pages" => total,
                            _ => return Err(p.new_custom_error::<(), ()>(())),
                        };
                        if p.try_parse(|p| p.expect_comma()).is_ok() {
                            let style = p.expect_ident()?;
                            if !style.eq_ignore_ascii_case("decimal") {
                                return Err(p.new_custom_error::<(), ()>(()));
                            }
                        }
                        p.expect_exhausted()?;
                        Ok(number)
                    })
                    .ok()?;
                result.push_str(&number.to_string());
            }
            _ => return None,
        }
    }
    Some(result)
}

pub(super) fn needs_continuations(rules: &[Rule]) -> bool {
    rules.iter().any(|rule| !rule.selectors.is_empty())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_scope_roots_keep_page_margin_resources_and_shadow_sheets_do_not() {
        for scope in [
            StylesheetScopeRoot::DocumentElement,
            StylesheetScopeRoot::Element(1),
            StylesheetScopeRoot::Constructed,
        ] {
            let mut input = super::super::tests::fixture();
            input.stylesheets = vec![
                "@page {margin:2cm;@top-left {content:\"Header\";background-image:url(data:image/png;base64,AA==)}}".into(),
                "@page {margin:0;@top-left {content:\"Shadow leak\";background-image:none}}".into(),
            ];
            input.stylesheet_scope_roots.insert(0, scope.clone());
            input.stylesheet_scope_roots.insert(1, scope);
            input.shadow_trees.stylesheet_scopes.insert(1, 99);
            let rules = read(&input);
            assert_eq!(rules.len(), 1, "only document page context is admitted");
            assert_eq!(cascade(&rules, "", 0, false)["margin-top"], "2cm");
            let margins = margin_styles(&rules, "", 0, false);
            assert_eq!(
                margin_content(&margins["top-left"]["content"], 1, 1).as_deref(),
                Some("Header")
            );
            assert!(margins["top-left"]["background-image"].contains("data:image/png"));
            // The isolated read above proves shadow rejection. Full formatting
            // below uses only the admitted document sheet and its native scope.
            input.stylesheets.pop();
            input.stylesheet_scope_roots.remove(&1);
            input.shadow_trees.stylesheet_scopes.clear();
            let printed = super::super::compute(
                &input,
                &LayoutLimits::default(),
                &PrintOptions {
                    background: true,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(
                printed
                    .pages
                    .iter()
                    .flat_map(|page| &page.generated_nodes)
                    .any(|node| {
                        node.text.iter().any(|text| text.text == "Header")
                            && node
                                .style
                                .get("background-image")
                                .is_some_and(|value| value.contains("data:image/png"))
                    })
            );
            assert!(
                !printed
                    .pages
                    .iter()
                    .flat_map(|page| &page.generated_nodes)
                    .flat_map(|node| &node.text)
                    .any(|text| text.text.contains("Shadow leak"))
            );
        }
    }
    #[test]
    fn margin_content_decodes_css_strings_and_uses_unfiltered_page_numbers() {
        assert_eq!(
            margin_content(r#""Page " counter(page) " / " counter(pages)"#, 3, 7).as_deref(),
            Some("Page 3 / 7")
        );
        assert_eq!(
            margin_content(r#""\41 \1f600 ""#, 1, 1).as_deref(),
            Some("A😀")
        );
        assert_eq!(margin_content("none", 1, 1), None);
        assert_eq!(margin_content("counter(chapter)", 1, 1), None);
    }
    #[test]
    fn native_page_cascade_respects_names_specificity_importance_and_print_media() {
        let mut input = super::super::tests::fixture();
        input.stylesheets=vec!["@page {margin: 1cm;size:A4} @page :first {margin-top:2cm} @page report:first {margin-top:3cm} @page {margin-left:4cm!important} @media screen {@page {size:A3}}".into()];
        let rules = read(&input);
        let generic = cascade(&rules, "", 0, false);
        let named = cascade(&rules, "report", 0, false);
        assert_eq!(generic["margin-top"], "2cm");
        assert_eq!(named["margin-top"], "3cm");
        let page = dimensions(
            validate_options(&PrintOptions::default()).unwrap(),
            &generic,
        )
        .unwrap();
        assert!((page.width - 210. * 96. / 25.4).abs() < 0.01);
        assert!((page.content.x - 4. * PX_PER_CM).abs() < 0.01);
    }
}
