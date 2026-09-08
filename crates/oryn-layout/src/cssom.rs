//! CSSOM declaration operations backed by the same Stylo parser as layout.
mod media_lists;
mod rule_mutations;
pub use media_lists::{MediaListEdit, MediaListMutation, mutate_media_list};
pub use rule_mutations::{RuleListEdit, RuleMutationError, mutate_rule_list};
use style::{
    context::QuirksMode,
    properties::declaration_block::{
        Importance, SourcePropertyDeclarationUpdate, parse_one_declaration_into,
    },
    properties::{PropertyId, SourcePropertyDeclaration, parse_style_attribute},
    stylesheets::{CssRuleType, Origin, UrlExtraData},
};
use style_traits::{ParsingMode, ToCss as _};

/// Builds import-rule metadata for a read-only CSSOM parse. Resource loading
/// belongs to the host stylesheet pipeline, never to property enumeration.
struct CssomImportLoader {
    base_url: UrlExtraData,
    quirks: QuirksMode,
}

impl style::stylesheets::StylesheetLoader for CssomImportLoader {
    fn request_stylesheet(
        &self,
        url: style::values::CssUrl,
        location: cssparser::SourceLocation,
        lock: &style::shared_lock::SharedRwLock,
        media: style::servo_arc::Arc<style::shared_lock::Locked<style::media_queries::MediaList>>,
        supports: Option<style::stylesheets::import_rule::ImportSupportsCondition>,
        layer: style::stylesheets::import_rule::ImportLayer,
    ) -> style::servo_arc::Arc<
        style::shared_lock::Locked<style::stylesheets::import_rule::ImportRule>,
    > {
        use style::{
            servo_arc::Arc,
            stylesheets::{
                AllowImportRules, Stylesheet,
                import_rule::{ImportRule, ImportSheet},
            },
        };
        // This transient child carries the media list needed for serialization.
        // It is not a loaded sheet and is never installed in the document.
        let base_url = url
            .url()
            .map(|resolved| UrlExtraData::from(resolved.as_ref().clone()))
            .unwrap_or_else(|| self.base_url.clone());
        let child = Stylesheet::from_str(
            "",
            base_url,
            Origin::Author,
            media,
            lock.clone(),
            None,
            None,
            self.quirks,
            AllowImportRules::No,
        );
        Arc::new(lock.wrap(ImportRule {
            url,
            stylesheet: ImportSheet::new(Arc::new(child)),
            supports,
            layer,
            source_location: location,
        }))
    }
}

/// A CSSOM rule view produced by Stylo, including its actual rule kind.
/// The binding layer assigns identity; it must not infer kinds or declaration
/// boundaries by splitting the serialized CSS text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleView {
    pub rule_type: u16,
    pub css_text: String,
    pub selector_text: Option<String>,
    pub declaration_text: Option<String>,
    pub children: Vec<RuleView>,
    pub properties: std::collections::BTreeMap<String, String>,
    pub conditions: Vec<(String, String)>,
    pub media_queries: Vec<String>,
}

pub fn read_stylesheet(
    source: &str,
    base_url: &str,
    quirks: bool,
    allow_imports: bool,
) -> Result<Vec<RuleView>, String> {
    use style::{
        media_queries::MediaList,
        servo_arc::Arc,
        shared_lock::SharedRwLock,
        stylesheets::{AllowImportRules, Stylesheet, StylesheetInDocument},
    };
    let lock = SharedRwLock::new();
    let loader = CssomImportLoader {
        base_url: UrlExtraData::from(url::Url::parse(base_url).map_err(|error| error.to_string())?),
        quirks: if quirks {
            QuirksMode::Quirks
        } else {
            QuirksMode::NoQuirks
        },
    };
    let sheet = Stylesheet::from_str(
        source,
        UrlExtraData::from(url::Url::parse(base_url).map_err(|error| error.to_string())?),
        Origin::Author,
        Arc::new(lock.wrap(MediaList::empty())),
        lock.clone(),
        Some(&loader),
        None,
        if quirks {
            QuirksMode::Quirks
        } else {
            QuirksMode::NoQuirks
        },
        if allow_imports {
            AllowImportRules::Yes
        } else {
            AllowImportRules::No
        },
    );
    let guard = sheet.shared_lock.read();
    let contents = sheet.contents(&guard);
    contents
        .rules
        .read_with(&guard)
        .0
        .iter()
        .map(|rule| rule_view(rule, &guard))
        .collect()
}

/// Project native nodes recursively while the stylesheet lock is held. In
/// particular, namespace-sensitive selectors are never reparsed in isolation.
fn rule_view(
    rule: &style::stylesheets::CssRule,
    guard: &style::shared_lock::SharedRwLockReadGuard,
) -> Result<RuleView, String> {
    use cssparser::ToCss;
    use style::{shared_lock::ToCssWithGuard, stylesheets::CssRule};
    let mut view = RuleView {
        rule_type: rule.rule_type() as u16,
        css_text: rule.to_css_string(guard).to_string(),
        children: rule
            .children(guard)
            .iter()
            .map(|child| rule_view(child, guard))
            .collect::<Result<_, _>>()?,
        ..RuleView::default()
    };
    let declaration = |block: &style::properties::PropertyDeclarationBlock| {
        let mut text = String::new();
        block.to_css(&mut text).map_err(|error| error.to_string())?;
        Ok::<_, String>(text)
    };
    match rule {
        CssRule::Style(rule) => {
            let rule = rule.read_with(guard);
            view.selector_text = Some(rule.selectors.to_css_string());
            view.declaration_text = Some(declaration(rule.block.read_with(guard))?);
        }
        CssRule::Page(rule) => {
            let rule = rule.read_with(guard);
            view.selector_text = Some(rule.selectors.to_css_string());
            view.declaration_text = Some(declaration(rule.block.read_with(guard))?);
        }
        CssRule::Media(rule) => {
            view.media_queries = rule
                .media_queries
                .read_with(guard)
                .media_queries
                .iter()
                .map(|query| query.to_css_string())
                .collect();
            view.properties.insert(
                "conditionText".into(),
                rule.media_queries.read_with(guard).to_css_string(),
            );
        }
        CssRule::Supports(rule) => {
            view.properties
                .insert("matches".into(), rule.enabled.to_string());
            view.properties
                .insert("conditionText".into(), rule.condition.to_css_string());
        }
        CssRule::Container(rule) => {
            view.properties
                .insert("conditionText".into(), rule.conditions.to_css_string());
            view.conditions = rule
                .conditions
                .0
                .iter()
                .map(|condition| {
                    (
                        condition.name().to_css_string(),
                        condition
                            .query_condition()
                            .map_or_else(String::new, |query| query.to_css_string()),
                    )
                })
                .collect();
            // CSS Conditional 5: the legacy attributes are empty for multiple conditions.
            // https://drafts.csswg.org/css-conditional-5/#the-csscontainerrule-interface
            let (name, query) = if view.conditions.len() == 1 {
                view.conditions[0].clone()
            } else {
                (String::new(), String::new())
            };
            view.properties.insert("containerName".into(), name);
            view.properties.insert("containerQuery".into(), query);
        }

        CssRule::Document(rule) => {
            view.properties
                .insert("conditionText".into(), rule.condition.to_css_string());
        }
        CssRule::Namespace(rule) => {
            view.properties.insert(
                "prefix".into(),
                rule.prefix
                    .as_ref()
                    .map_or_else(String::new, |prefix| prefix.0.to_string()),
            );
            view.properties
                .insert("namespaceURI".into(), rule.url.to_string());
        }
        CssRule::Keyframes(rule) => {
            let rule = rule.read_with(guard);
            view.properties
                .insert("name".into(), rule.name.as_atom().to_string());
            view.children = rule
                .keyframes
                .iter()
                .map(|child| {
                    let child = child.read_with(guard);
                    Ok(RuleView {
                        rule_type: CssRuleType::Keyframe as u16,
                        css_text: child.to_css_string(guard).to_string(),
                        declaration_text: Some(declaration(child.block.read_with(guard))?),
                        properties: std::collections::BTreeMap::from([(
                            "keyText".into(),
                            child.selector.to_css_string(),
                        )]),
                        ..RuleView::default()
                    })
                })
                .collect::<Result<_, String>>()?;
        }
        CssRule::Margin(rule) => {
            view.properties.insert("name".into(), rule.name().into());
            view.declaration_text = Some(declaration(rule.block.read_with(guard))?);
        }
        CssRule::FontFace(rule) => {
            view.declaration_text = Some(rule.read_with(guard).descriptors.to_css_string());
        }
        CssRule::CounterStyle(rule) => {
            let rule = rule.read_with(guard);
            view.properties
                .insert("name".into(), rule.name().0.to_string());
            view.declaration_text = Some(rule.descriptors().to_css_string());
        }
        CssRule::FontFeatureValues(rule) => {
            view.properties.insert(
                "fontFamily".into(),
                rule.family_names
                    .iter()
                    .map(|name| name.to_css_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        CssRule::FontPaletteValues(rule) => {
            view.properties
                .insert("name".into(), rule.name.0.to_string());
            view.properties.insert(
                "fontFamily".into(),
                rule.family_names
                    .iter()
                    .map(|name| name.to_css_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        CssRule::LayerBlock(rule) => {
            view.properties.insert(
                "name".into(),
                rule.name
                    .as_ref()
                    .map_or_else(String::new, |name| name.to_css_string()),
            );
        }
        CssRule::Scope(rule) => {
            if let Some(start) = &rule.bounds.start {
                view.properties
                    .insert("start".into(), start.to_css_string());
            }
            if let Some(end) = &rule.bounds.end {
                view.properties.insert("end".into(), end.to_css_string());
            }
        }
        CssRule::PositionTry(rule) => {
            let rule = rule.read_with(guard);
            view.properties
                .insert("name".into(), rule.name.0.to_string());
            view.declaration_text = Some(declaration(rule.block.read_with(guard))?);
        }
        CssRule::NestedDeclarations(rule) => {
            view.declaration_text =
                Some(declaration(rule.read_with(guard).block.read_with(guard))?);
        }
        CssRule::Import(rule) => {
            let rule = rule.read_with(guard);
            if let Some(url) = rule.url.url() {
                view.properties.insert("href".into(), url.to_string());
            }
        }
        _ => {}
    }
    Ok(view)
}

/// Mutate a page selector through Stylo, preserving declarations and nested rules.
/// Invalid selectors leave the rule untouched, as required by CSSOM.
pub fn set_page_rule_selector(
    source: &str,
    path: &[usize],
    selector: &str,
    base_url: &str,
    quirks: bool,
) -> Result<Option<Vec<RuleView>>, String> {
    use style::{
        media_queries::MediaList,
        parser::{Parse, ParserContext},
        servo_arc::Arc,
        shared_lock::SharedRwLock,
        stylesheets::{AllowImportRules, CssRule, PageSelectors, Stylesheet, StylesheetInDocument},
    };
    let url_data = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let quirks = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let context = ParserContext::new(
        Origin::Author,
        &url_data,
        Some(CssRuleType::Page),
        ParsingMode::DEFAULT,
        quirks,
        Default::default(),
        None,
        None,
        Default::default(),
    );
    let mut input = cssparser::ParserInput::new(selector);
    let mut parser = cssparser::Parser::new(&mut input);
    let selectors = if parser.is_exhausted() {
        PageSelectors::default()
    } else {
        match parser.parse_entirely(|input| PageSelectors::parse(&context, input)) {
            Ok(selectors) => selectors,
            Err(_) => return Ok(None),
        }
    };
    let lock = SharedRwLock::new();
    let loader = CssomImportLoader {
        base_url: url_data.clone(),
        quirks,
    };
    let sheet = Stylesheet::from_str(
        source,
        url_data,
        Origin::Author,
        Arc::new(lock.wrap(MediaList::empty())),
        lock.clone(),
        Some(&loader),
        None,
        quirks,
        AllowImportRules::Yes,
    );
    let rule = {
        let guard = lock.read();
        let mut rules = sheet.contents(&guard).rules.read_with(&guard).0.as_slice();
        let (&last, ancestors) = path.split_last().ok_or("Empty CSS rule path")?;
        for &index in ancestors {
            let rule = rules.get(index).ok_or("CSS rule path out of bounds")?;
            if matches!(rule, CssRule::Import(_)) {
                return Err("Import sheet requires its own mutation context".into());
            }
            rules = rule.children(&guard);
        }
        match rules.get(last).ok_or("CSS rule path out of bounds")? {
            CssRule::Page(rule) => rule.clone(),
            _ => return Err("CSS rule is not a page rule".into()),
        }
    };
    rule.write_with(&mut lock.write()).selectors = selectors;
    let guard = lock.read();
    Ok(Some(
        sheet
            .contents(&guard)
            .rules
            .read_with(&guard)
            .0
            .iter()
            .map(|rule| rule_view(rule, &guard))
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

/// Mutate one standalone page rule through the same full-context transaction.
pub fn set_page_selector(
    source: &str,
    selector: &str,
    base_url: &str,
    quirks: bool,
) -> Result<Option<RuleView>, String> {
    let rules = read_stylesheet(source, base_url, quirks, false)?;
    if rules.len() != 1 || rules[0].rule_type != 6 {
        return Ok(None);
    }
    Ok(
        set_page_rule_selector(&rules[0].css_text, &[0], selector, base_url, quirks)?
            .and_then(|rules| rules.into_iter().next()),
    )
}

/// A page declaration edit, including the old value returned by removeProperty.
pub struct PageDeclarationMutation {
    pub previous_value: String,
    pub rule: Option<RuleView>,
}

/// Set a style selector using the original sheet namespace and nesting context.
pub fn set_style_rule_selector(
    source: &str,
    path: &[usize],
    selector: &str,
    base_url: &str,
    quirks: bool,
) -> Result<Option<Vec<RuleView>>, String> {
    use style::{
        media_queries::MediaList,
        servo_arc::Arc,
        shared_lock::SharedRwLock,
        stylesheets::{AllowImportRules, CssRule, Stylesheet, StylesheetInDocument},
    };
    let url_data = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let quirks_mode = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let lock = SharedRwLock::new();
    let loader = CssomImportLoader {
        base_url: url_data.clone(),
        quirks: quirks_mode,
    };
    let sheet = Stylesheet::from_str(
        source,
        url_data.clone(),
        Origin::Author,
        Arc::new(lock.wrap(MediaList::empty())),
        lock.clone(),
        Some(&loader),
        None,
        quirks_mode,
        AllowImportRules::Yes,
    );
    let (rule, selectors) = {
        use selectors::{SelectorList, parser::ParseRelative};
        let guard = lock.read();
        let contents = sheet.contents(&guard);
        let mut rules = contents.rules.read_with(&guard).0.as_slice();
        let (&last, ancestors) = path.split_last().ok_or("Empty CSS rule path")?;
        let mut relative = ParseRelative::No;
        for &index in ancestors {
            let rule = rules.get(index).ok_or("CSS rule path out of bounds")?;
            match rule {
                CssRule::Style(_) => relative = ParseRelative::ForNesting,
                CssRule::Scope(_) => relative = ParseRelative::ForScope,
                _ => {}
            }
            rules = rule.children(&guard);
        }
        let CssRule::Style(rule) = rules.get(last).ok_or("CSS rule path out of bounds")? else {
            return Err("CSS rule is not a style rule".into());
        };
        let selector_parser = style::selector_parser::SelectorParser {
            stylesheet_origin: Origin::Author,
            namespaces: &contents.namespaces,
            url_data: &url_data,
            for_supports_rule: false,
        };
        let mut input = cssparser::ParserInput::new(selector);
        let mut parser = cssparser::Parser::new(&mut input);
        let selectors = match parser
            .parse_entirely(|input| SelectorList::parse(&selector_parser, input, relative))
        {
            Ok(selectors) => selectors,
            Err(_) => return Ok(None),
        };
        (rule.clone(), selectors)
    };
    rule.write_with(&mut lock.write()).selectors = selectors;
    let guard = lock.read();
    let rules = sheet
        .contents(&guard)
        .rules
        .read_with(&guard)
        .0
        .iter()
        .map(|rule| rule_view(rule, &guard))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(rules))
}

/// Result of a declaration edit in its original stylesheet context.
#[derive(Debug)]
pub struct RuleDeclarationMutation {
    pub previous_value: String,
    /// None means that the declaration operation made no change.
    pub rules: Option<Vec<RuleView>>,
}

/// Edit a native declaration block at a rule path. Parsing the complete sheet
/// preserves namespace declarations, URL resolution and nested selector context.
/// Paths are transaction-local positions; bindings retain their own rule identities.
pub fn mutate_rule_declaration(
    source: &str,
    path: &[usize],
    property: Option<&str>,
    value: &str,
    priority: &str,
    base_url: &str,
    quirks: bool,
) -> Result<RuleDeclarationMutation, String> {
    use style::{
        media_queries::MediaList,
        servo_arc::Arc,
        shared_lock::SharedRwLock,
        stylesheets::{AllowImportRules, CssRule, Stylesheet, StylesheetInDocument},
    };
    let url_data = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let quirks_mode = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let lock = SharedRwLock::new();
    let loader = CssomImportLoader {
        base_url: url_data.clone(),
        quirks: quirks_mode,
    };
    let sheet = Stylesheet::from_str(
        source,
        url_data.clone(),
        Origin::Author,
        Arc::new(lock.wrap(MediaList::empty())),
        lock.clone(),
        Some(&loader),
        None,
        quirks_mode,
        AllowImportRules::Yes,
    );
    let (block, kind, declarations) = {
        let guard = lock.read();
        let contents = sheet.contents(&guard);
        let mut rules = contents.rules.read_with(&guard).0.as_slice();
        let (&last, ancestors) = path.split_last().ok_or("Empty CSS rule path")?;
        let (block, kind) = 'target: {
            for (depth, &index) in ancestors.iter().enumerate() {
                let rule = rules.get(index).ok_or("CSS rule path out of bounds")?;
                if let CssRule::Keyframes(rule) = rule {
                    if depth + 1 != ancestors.len() {
                        return Err("Keyframe rules cannot contain child rules".into());
                    }
                    let keyframes = rule.read_with(&guard);
                    let keyframe = keyframes
                        .keyframes
                        .get(last)
                        .ok_or("CSS keyframe path out of bounds")?;
                    break 'target (
                        keyframe.read_with(&guard).block.clone(),
                        CssRuleType::Keyframe,
                    );
                }
                rules = rule.children(&guard);
            }
            let rule = rules.get(last).ok_or("CSS rule path out of bounds")?;
            let block = match rule {
                CssRule::Style(rule) => rule.read_with(&guard).block.clone(),
                CssRule::Page(rule) => rule.read_with(&guard).block.clone(),
                CssRule::Margin(rule) => rule.block.clone(),
                CssRule::PositionTry(rule) => rule.read_with(&guard).block.clone(),
                CssRule::NestedDeclarations(rule) => rule.read_with(&guard).block.clone(),
                _ => return Err("CSS rule does not contain a property declaration block".into()),
            };
            (block, rule.rule_type())
        };
        let mut declarations = String::new();
        block
            .read_with(&guard)
            .to_css(&mut declarations)
            .map_err(|e| e.to_string())?;
        (block, kind, declarations)
    };
    let mutation = match property {
        Some(property) => mutate_declaration_in_rule(
            &declarations,
            property,
            value,
            priority,
            base_url,
            quirks,
            kind,
        )?,
        None => DeclarationMutation {
            previous_value: String::new(),
            css_text: Some(value.into()),
        },
    };
    let Some(text) = mutation.css_text else {
        return Ok(RuleDeclarationMutation {
            previous_value: mutation.previous_value,
            rules: None,
        });
    };
    *block.write_with(&mut lock.write()) =
        parse_style_attribute(&text, &url_data, None, quirks_mode, kind);
    let guard = lock.read();
    let rules = sheet
        .contents(&guard)
        .rules
        .read_with(&guard)
        .0
        .iter()
        .map(|rule| rule_view(rule, &guard))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RuleDeclarationMutation {
        previous_value: mutation.previous_value,
        rules: Some(rules),
    })
}

/// Edit the native declaration block without reconstructing the page's nested rules.
pub fn mutate_page_declaration(
    source: &str,
    property: &str,
    value: &str,
    priority: &str,
    base_url: &str,
    quirks: bool,
) -> Result<PageDeclarationMutation, String> {
    edit_page_declaration_block(source, Some(property), value, priority, base_url, quirks)
}

/// Replace only a page rule's declarations; its margin rules remain intact.
pub fn replace_page_declarations(
    source: &str,
    declarations: &str,
    base_url: &str,
    quirks: bool,
) -> Result<PageDeclarationMutation, String> {
    edit_page_declaration_block(source, None, declarations, "", base_url, quirks)
}

fn edit_page_declaration_block(
    source: &str,
    property: Option<&str>,
    value: &str,
    priority: &str,
    base_url: &str,
    quirks: bool,
) -> Result<PageDeclarationMutation, String> {
    use style::{
        media_queries::MediaList,
        servo_arc::Arc,
        shared_lock::{SharedRwLock, ToCssWithGuard},
        stylesheets::{AllowImportRules, CssRule, Stylesheet, StylesheetInDocument},
    };
    let url_data = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let quirks_mode = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let lock = SharedRwLock::new();
    let sheet = Stylesheet::from_str(
        source,
        url_data.clone(),
        Origin::Author,
        Arc::new(lock.wrap(MediaList::empty())),
        lock.clone(),
        None,
        None,
        quirks_mode,
        AllowImportRules::No,
    );
    let (rule, declarations) = {
        let guard = lock.read();
        let rules = &sheet.contents(&guard).rules.read_with(&guard).0;
        let [CssRule::Page(rule)] = rules.as_slice() else {
            return Err("Expected one native page rule".into());
        };
        let mut text = String::new();
        rule.read_with(&guard)
            .block
            .read_with(&guard)
            .to_css(&mut text)
            .map_err(|e| e.to_string())?;
        (rule.clone(), text)
    };
    let mutation = match property {
        Some(property) => mutate_declaration_in_rule(
            &declarations,
            property,
            value,
            priority,
            base_url,
            quirks,
            CssRuleType::Page,
        )?,
        None => DeclarationMutation {
            previous_value: String::new(),
            css_text: Some(value.into()),
        },
    };
    let Some(text) = mutation.css_text else {
        return Ok(PageDeclarationMutation {
            previous_value: mutation.previous_value,
            rule: None,
        });
    };
    let block = parse_style_attribute(&text, &url_data, None, quirks_mode, CssRuleType::Page);
    rule.write_with(&mut lock.write()).block = Arc::new(lock.wrap(block));
    let serialized = {
        let guard = lock.read();
        rule.read_with(&guard).to_css_string(&guard).to_string()
    };
    let view = read_stylesheet(&serialized, base_url, quirks, false)?
        .into_iter()
        .next();
    Ok(PageDeclarationMutation {
        previous_value: mutation.previous_value,
        rule: view,
    })
}

/// Result of a declaration mutation. `None` leaves the attribute untouched,
/// including its original spelling and any mutation-observer bookkeeping.
#[derive(Debug, PartialEq, Eq)]
pub struct DeclarationMutation {
    pub previous_value: String,
    pub css_text: Option<String>,
}

/// Set or remove an authored declaration using CSSOM property parsing rules.
pub fn mutate_declaration(
    css_text: &str,
    property: &str,
    value: &str,
    priority: &str,
    base_url: &str,
    quirks: bool,
) -> Result<DeclarationMutation, String> {
    mutate_declaration_in_rule(
        css_text,
        property,
        value,
        priority,
        base_url,
        quirks,
        CssRuleType::Style,
    )
}

fn mutate_declaration_in_rule(
    css_text: &str,
    property: &str,
    value: &str,
    priority: &str,
    base_url: &str,
    quirks: bool,
    rule_type: CssRuleType,
) -> Result<DeclarationMutation, String> {
    let mut result = DeclarationMutation {
        previous_value: String::new(),
        css_text: None,
    };
    let Ok(property) = PropertyId::parse_enabled_for_all_content(property) else {
        return Ok(result);
    };
    let url_data = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let quirks_mode = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let mut block = parse_style_attribute(css_text, &url_data, None, quirks_mode, rule_type);
    block
        .property_value_to_css(&property, &mut result.previous_value)
        .map_err(|e| e.to_string())?;
    if value.is_empty() {
        let Some(first) = block.first_declaration_to_remove(&property) else {
            return Ok(result);
        };
        block.remove_property(&property, first);
    } else {
        let importance = if priority.is_empty() {
            Importance::Normal
        } else if priority.eq_ignore_ascii_case("important") {
            Importance::Important
        } else {
            return Ok(result);
        };
        let mut declarations = SourcePropertyDeclaration::default();
        if parse_one_declaration_into(
            &mut declarations,
            property,
            value,
            Origin::Author,
            &url_data,
            None,
            ParsingMode::DEFAULT,
            quirks_mode,
            rule_type,
        )
        .is_err()
        {
            return Ok(result);
        }
        let mut updates = SourcePropertyDeclarationUpdate::default();
        if !block.prepare_for_update(&declarations, importance, &mut updates) {
            return Ok(result);
        }
        block.update(declarations.drain(), importance, &mut updates);
    }
    let mut serialized = String::new();
    block.to_css(&mut serialized).map_err(|e| e.to_string())?;
    result.css_text = Some(serialized);
    Ok(result)
}

/// A serialized declaration block and a property queried from that block.
#[derive(Debug, PartialEq, Eq)]
pub struct DeclarationView {
    pub css_text: String,
    pub names: Vec<String>,
    pub value: String,
    pub important: bool,
}

pub fn read_declaration(
    css_text: &str,
    property: &str,
    base_url: &str,
    quirks: bool,
) -> Result<DeclarationView, String> {
    read_declaration_in_rule(css_text, property, base_url, quirks, CssRuleType::Style)
}

/// Read page declarations in their native parsing context.
pub fn read_page_declaration(
    css_text: &str,
    property: &str,
    base_url: &str,
    quirks: bool,
) -> Result<DeclarationView, String> {
    read_declaration_in_rule(css_text, property, base_url, quirks, CssRuleType::Page)
}

fn read_declaration_in_rule(
    css_text: &str,
    property: &str,
    base_url: &str,
    quirks: bool,
    rule_type: CssRuleType,
) -> Result<DeclarationView, String> {
    let url_data = UrlExtraData::from(url::Url::parse(base_url).map_err(|e| e.to_string())?);
    let quirks = if quirks {
        QuirksMode::Quirks
    } else {
        QuirksMode::NoQuirks
    };
    let block = parse_style_attribute(css_text, &url_data, None, quirks, rule_type);
    let mut result = DeclarationView {
        css_text: String::new(),
        names: block
            .declaration_importance_iter()
            .map(|(declaration, _)| declaration.id().name().into_owned())
            .collect(),
        value: String::new(),
        important: false,
    };
    block
        .to_css(&mut result.css_text)
        .map_err(|e| e.to_string())?;
    if let Ok(property) = PropertyId::parse_enabled_for_all_content(property) {
        block
            .property_value_to_css(&property, &mut result.value)
            .map_err(|e| e.to_string())?;
        result.important = block.property_priority(&property).important();
    }
    Ok(result)
}

/// Supported property spellings and their canonical (unaliasing) targets.
/// Both enumeration and enablement come from Stylo's generated property table.
pub fn supported_properties() -> Vec<(String, String)> {
    style::properties::NonCustomPropertyId::iter()
        .filter_map(|id| {
            PropertyId::parse_enabled_for_all_content(id.name())
                .ok()
                .map(|_| (id.name().to_owned(), id.unaliased().name().to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_selector_transaction_preserves_full_sheet_and_nested_margin_rules() {
        let base = "https://example.test/styles/main.css";
        let source = "@namespace svg url(http://www.w3.org/2000/svg); @media print { @page chapter { margin-top: 10px; @top-left { content: 'heading'; } } svg|rect { color: red; } } aside { color: blue; }";
        let before = read_stylesheet(source, base, false, true).unwrap();
        let after = set_page_rule_selector(source, &[1, 0], "chapter:left", base, false)
            .unwrap()
            .unwrap();
        assert_eq!(after.len(), 3);
        assert_eq!(after[0], before[0]);
        assert_eq!(after[2], before[2]);
        assert_eq!(after[1].children[1], before[1].children[1]);
        let page = &after[1].children[0];
        assert_eq!(page.selector_text.as_deref(), Some("chapter:left"));
        assert_eq!(
            page.declaration_text,
            before[1].children[0].declaration_text
        );
        assert_eq!(page.children, before[1].children[0].children);
        assert!(after[1].css_text.contains("chapter:left"));
        assert!(!before[1].css_text.contains("chapter:left"));
    }

    #[test]
    fn page_selector_transaction_rejects_invalid_selectors_and_validates_paths() {
        let base = "https://example.test/";
        let source = "@media print { @page chapter { margin: 1cm; } }";
        for invalid in [
            "div > span",
            ":unknown",
            "chapter,",
            "chapter { margin: 0 }",
        ] {
            assert!(
                set_page_rule_selector(source, &[0, 0], invalid, base, false)
                    .unwrap()
                    .is_none(),
                "{invalid}"
            );
        }
        for path in [&[][..], &[0][..], &[0, 1][..]] {
            assert!(set_page_rule_selector(source, path, "chapter:left", base, false).is_err());
        }
        let after = set_page_rule_selector(source, &[0, 0], "", base, false)
            .unwrap()
            .unwrap();
        assert_eq!(after[0].children[0].selector_text.as_deref(), Some(""));
        assert_eq!(
            after[0].children[0].declaration_text.as_deref(),
            Some("margin: 1cm;")
        );
    }

    #[test]
    fn style_selector_edits_preserve_namespace_and_nested_context() {
        let source = r#"@namespace svg url("http://www.w3.org/2000/svg");
            @media screen { svg|rect { color: red; & > svg|circle { opacity: .5; } } }
            aside { color: green; }"#;
        let base = "https://example.test/";
        let before = read_stylesheet(source, base, false, true).unwrap();
        let after =
            set_style_rule_selector(source, &[1, 0], "svg|circle[data-token=\"{\"]", base, false)
                .unwrap()
                .unwrap();
        assert_eq!(
            after[1].children[0].selector_text.as_deref(),
            Some("svg|circle[data-token=\"{\"]")
        );
        assert_eq!(
            after[1].children[0].children,
            before[1].children[0].children
        );
        assert_eq!(
            after[1].children[0].declaration_text,
            before[1].children[0].declaration_text
        );
        assert_eq!(after[0], before[0]);
        assert_eq!(after[2], before[2]);
        let after = set_style_rule_selector(source, &[1, 0, 0], "> svg|path", base, false)
            .unwrap()
            .unwrap();
        assert!(
            after[1].children[0].children[0]
                .selector_text
                .as_ref()
                .unwrap()
                .contains("svg|path")
        );
        assert_eq!(
            after[1].children[0].children[0].declaration_text,
            before[1].children[0].children[0].declaration_text
        );
        for invalid in ["", "missing|rect", "a { color: blue }", "a,"] {
            assert!(
                set_style_rule_selector(source, &[1, 0], invalid, base, false)
                    .unwrap()
                    .is_none(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn keyframe_declaration_edits_preserve_siblings_and_key_text() {
        let source = "@keyframes pulse { from { opacity: 0; } 50%, to { opacity: 1; } }";
        let base = "https://example.test/";
        let before = read_stylesheet(source, base, false, true).unwrap();
        let edited =
            mutate_rule_declaration(source, &[0, 1], Some("opacity"), "0.75", "", base, false)
                .unwrap()
                .rules
                .unwrap();
        assert_eq!(edited[0].children[0], before[0].children[0]);
        assert_eq!(
            edited[0].children[1].properties,
            before[0].children[1].properties
        );
        assert_eq!(
            edited[0].children[1].declaration_text.as_deref(),
            Some("opacity: 0.75;")
        );
        assert!(
            mutate_rule_declaration(source, &[0, 1, 0], None, "opacity: 0", "", base, false)
                .is_err()
        );
    }

    #[test]
    fn rule_declaration_edits_preserve_whole_sheet_context() {
        let source = r#"@namespace svg url("http://www.w3.org/2000/svg");
            @media screen { svg|rect { color: red; & > svg|circle { opacity: .5; } } }
            @page chapter { margin: 1px; @top-left { content: "a{b}"; } }
            aside { color: green; }"#;
        let base = "https://example.test/css/main.css";
        let before = read_stylesheet(source, base, false, true).unwrap();
        let edited = mutate_rule_declaration(
            source,
            &[1, 0],
            Some("color"),
            "blue",
            "important",
            base,
            false,
        )
        .unwrap();
        assert_eq!(edited.previous_value, "red");
        let after = edited.rules.unwrap();
        assert_eq!(after[0], before[0]);
        assert_eq!(after[2..], before[2..]);
        assert_eq!(
            after[1].children[0].selector_text,
            before[1].children[0].selector_text
        );
        assert_eq!(
            after[1].children[0].children,
            before[1].children[0].children
        );
        assert_eq!(
            after[1].children[0].declaration_text.as_deref(),
            Some("color: blue !important;")
        );
        let edited =
            mutate_rule_declaration(source, &[1, 0, 0], None, "opacity: .75", "", base, false)
                .unwrap()
                .rules
                .unwrap();
        assert_eq!(
            edited[1].children[0].children[0]
                .declaration_text
                .as_deref(),
            Some("opacity: 0.75;")
        );
        assert_eq!(
            edited[1].children[0].declaration_text,
            before[1].children[0].declaration_text
        );
        let edited = mutate_rule_declaration(
            source,
            &[2, 0],
            Some("content"),
            "\"new{value}\"",
            "",
            base,
            false,
        )
        .unwrap()
        .rules
        .unwrap();
        assert_eq!(
            edited[2].children[0].declaration_text.as_deref(),
            Some("content: \"new{value}\";")
        );
        assert_eq!(edited[2].declaration_text, before[2].declaration_text);
        assert_eq!(edited[0..2], before[0..2]);
        assert!(
            mutate_rule_declaration(
                source,
                &[1, 0],
                Some("color"),
                "not-a-color",
                "",
                base,
                false
            )
            .unwrap()
            .rules
            .is_none()
        );
        for path in [&[][..], &[99][..], &[1, 99][..], &[0][..]] {
            assert!(
                mutate_rule_declaration(source, path, None, "color: red", "", base, false).is_err()
            );
        }
    }

    #[test]
    fn native_rule_tree_preserves_nested_context_and_descriptor_blocks() {
        let rules = read_stylesheet(
            r#"
            @namespace svg url("http://www.w3.org/2000/svg");
            @media screen { @supports (color: red) {
                svg|rect { color: red; & > svg|circle { opacity: .5; } }
            } }
            @keyframes "two words" { from { opacity: 0; } 50%, to { opacity: 1; } }
            @page chapter { margin: 1px; @top-left { content: "a{b}"; } }
            @font-face { font-family: "Tree"; src: url(font.woff2); }
            @layer theme { a { color: blue; } }
            @container card (width > 10px) { b { color: green; } }
            @counter-style custom { system: cyclic; symbols: "x"; suffix: " "; }
        "#,
            "https://example.test/styles/main.css",
            false,
            true,
        )
        .unwrap();
        assert_eq!(
            rules.iter().map(|r| r.rule_type).collect::<Vec<_>>(),
            [10, 4, 7, 6, 5, 16, 18, 11]
        );
        assert_eq!(rules[0].properties["prefix"], "svg");
        assert_eq!(
            rules[0].properties["namespaceURI"],
            "http://www.w3.org/2000/svg"
        );
        assert_eq!(rules[1].properties["conditionText"], "screen");
        let supports = &rules[1].children[0];
        assert_eq!(supports.rule_type, 12);
        assert_eq!(supports.properties["conditionText"], "(color: red)");
        let style = &supports.children[0];
        assert_eq!(style.selector_text.as_deref(), Some("svg|rect"));
        assert_eq!(style.declaration_text.as_deref(), Some("color: red;"));
        assert_eq!(style.children.len(), 1);
        assert!(
            style.children[0]
                .selector_text
                .as_ref()
                .unwrap()
                .contains("svg|circle")
        );
        assert_eq!(rules[2].properties["name"], "two words");
        assert_eq!(rules[2].children.len(), 2);
        assert_eq!(rules[2].children[0].rule_type, 8);
        assert_eq!(rules[2].children[0].properties["keyText"], "0%");
        assert_eq!(
            rules[2].children[1].declaration_text.as_deref(),
            Some("opacity: 1;")
        );
        assert_eq!(rules[3].children.len(), 1);
        assert_eq!(rules[3].children[0].rule_type, 9);
        assert_eq!(rules[3].children[0].properties["name"], "top-left");
        assert_eq!(
            rules[3].children[0].declaration_text.as_deref(),
            Some("content: \"a{b}\";")
        );
        assert!(
            rules[4]
                .declaration_text
                .as_ref()
                .unwrap()
                .contains("font-family")
        );
        assert_eq!(rules[5].properties["name"], "theme");
        assert_eq!(rules[5].children.len(), 1);
        assert!(rules[6].properties["conditionText"].starts_with("card "));
        assert_eq!(rules[6].children.len(), 1);
        assert_eq!(rules[6].properties["containerName"], "card");
        assert_eq!(rules[6].properties["containerQuery"], "(width > 10px)");
        assert_eq!(rules[7].properties["name"], "custom");
        assert!(
            rules[7]
                .declaration_text
                .as_ref()
                .unwrap()
                .contains("symbols:")
        );
        let multiple = read_stylesheet(
            "@container sidebar (width > 1px), (height > 2px) {}",
            "https://example.test/",
            false,
            true,
        )
        .unwrap();
        assert_eq!(multiple.len(), 1);
        assert_eq!(multiple[0].conditions.len(), 2);
        assert_eq!(multiple[0].properties["containerName"], "");
        assert_eq!(multiple[0].properties["containerQuery"], "");
    }

    #[test]
    fn page_declaration_replacement_preserves_nested_rules_and_reads_priority() {
        let source = "@page named { margin: 1cm; @bottom-right { content: 'footer'; } }";
        let edited = replace_page_declarations(
            source,
            "margin-left: 3cm !important; invalid: value",
            "https://example.test/",
            false,
        )
        .unwrap()
        .rule
        .unwrap();
        assert_eq!(edited.selector_text.as_deref(), Some("named"));
        assert!(edited.css_text.contains("footer"));
        let read = read_page_declaration(
            edited.declaration_text.as_deref().unwrap(),
            "margin-left",
            "https://example.test/",
            false,
        )
        .unwrap();
        assert_eq!(read.value, "3cm");
        assert!(read.important);
        assert!(!read.css_text.contains("invalid"));
        let empty = replace_page_declarations(&edited.css_text, "", "https://example.test/", false)
            .unwrap()
            .rule
            .unwrap();
        assert_eq!(empty.declaration_text.as_deref(), Some(""));
        assert!(empty.css_text.contains("footer"));
    }

    #[test]
    fn page_declaration_mutation_keeps_selectors_and_nested_margin_content() {
        let source = "@page named:left { margin: 1cm; @top-left { content: 'heading'; } }";
        let edited = mutate_page_declaration(
            source,
            "margin-left",
            "2cm",
            "important",
            "https://example.test/",
            false,
        )
        .unwrap()
        .rule
        .unwrap();
        assert_eq!(edited.selector_text.as_deref(), Some("named:left"));
        assert!(edited.css_text.contains("heading"));
        assert!(edited.css_text.contains("2cm !important"));
        let removed = mutate_page_declaration(
            &edited.css_text,
            "margin-left",
            "",
            "",
            "https://example.test/",
            false,
        )
        .unwrap();
        assert_eq!(removed.previous_value, "2cm");
        assert!(removed.rule.unwrap().css_text.contains("heading"));
        assert!(
            mutate_page_declaration(
                source,
                "margin",
                "invalid",
                "",
                "https://example.test/",
                false
            )
            .unwrap()
            .rule
            .is_none()
        );
    }

    #[test]
    fn page_selector_mutation_validates_and_preserves_rule_contents() {
        let source = "@page original { margin: 1cm; @top-left { content: 'heading'; } }";
        for (selector, expected) in [("named:First", "named:first"), (":left", ":left"), ("", "")] {
            let rule = set_page_selector(source, selector, "https://example.test/", false)
                .unwrap()
                .unwrap();
            assert_eq!(rule.selector_text.as_deref(), Some(expected));
            assert!(rule.css_text.contains("margin:"));
            assert!(rule.css_text.contains("@top-left"));
            assert!(rule.css_text.contains("heading"));
        }
        for selector in [
            "named :first",
            ":first :left",
            ":unknown",
            "named {}",
            "named,",
            "named; @page other",
        ] {
            assert!(
                set_page_selector(source, selector, "https://example.test/", false)
                    .unwrap()
                    .is_none(),
                "{selector}"
            );
        }
    }

    #[test]
    fn page_rule_views_preserve_page_selectors_and_declarations() {
        let rules = read_stylesheet(
            "@page {} @page report:First { margin: 1cm; }",
            "https://example.test/",
            false,
            true,
        )
        .unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].rule_type, CssRuleType::Page as u16);
        assert_eq!(rules[0].selector_text.as_deref(), Some(""));
        assert_eq!(rules[0].css_text, "@page { }");
        assert_eq!(rules[1].selector_text.as_deref(), Some("report:first"));
        assert!(
            rules[1]
                .declaration_text
                .as_deref()
                .unwrap()
                .contains("margin")
        );
    }

    #[test]
    fn cssom_import_readback_retains_conditions_without_fetching() {
        for condition in ["display: block", "(display: flex)", "(unknown: value)"] {
            let source = format!(
                "@import url('unloaded.css') layer(theme) supports({condition}) screen; div {{ color: red; }}"
            );
            let rules =
                read_stylesheet(&source, "https://example.test/main.css", false, true).unwrap();
            assert_eq!(rules.len(), 2, "{condition}");
            assert_eq!(rules[0].rule_type, CssRuleType::Import as u16);
            for part in ["unloaded.css", "layer(theme)", "supports(", "screen"] {
                assert!(rules[0].css_text.contains(part), "{}", rules[0].css_text);
            }
        }
    }

    #[test]
    fn svg_resource_declarations_survive_stylo_round_trip() {
        for property in ["filter", "marker-start", "marker-mid", "marker-end"] {
            let source = format!("{property}: url(https://example.test/resource.svg#target)");
            let view = read_declaration(&source, property, "https://example.test/", false).unwrap();
            assert!(
                view.value.contains("resource.svg#target"),
                "{property}: {view:?}"
            );
            let mutation = mutate_declaration(
                "",
                property,
                "url(https://example.test/next.svg#target)",
                "important",
                "https://example.test/",
                false,
            )
            .unwrap();
            let changed = read_declaration(
                mutation.css_text.as_deref().unwrap(),
                property,
                "https://example.test/",
                false,
            )
            .unwrap();
            assert!(changed.important, "{property}");
            assert!(changed.value.contains("next.svg#target"), "{property}");
        }
    }

    #[test]
    fn stylesheet_replacement_discards_imports_using_the_parser() {
        let rules = read_stylesheet(
            r#"@import url('ignored.css'); @\69mport 'also-ignored.css'; div { color: red; }"#,
            "https://example.test/main.css",
            false,
            false,
        )
        .unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector_text.as_deref(), Some("div"));
    }

    #[test]
    fn stylesheet_views_use_parser_boundaries_and_declaration_validation() {
        let rules = read_stylesheet(
            r#"/* } */ .a, .b { --text: "};:"; width: -1px; margin: 1px 2px; }
               @media (min-width: 10px) { .nested { color: red; } }
               #last { height: 3px; }"#,
            "https://example.test/styles/main.css",
            false,
            true,
        )
        .unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].rule_type, CssRuleType::Style as u16);
        assert_eq!(rules[0].selector_text.as_deref(), Some(".a, .b"));
        let declaration = read_declaration(
            rules[0].declaration_text.as_deref().unwrap(),
            "width",
            "about:blank",
            false,
        )
        .unwrap();
        assert_eq!(declaration.value, "");
        assert!(declaration.css_text.contains("};:"));
        assert_eq!(rules[1].rule_type, CssRuleType::Media as u16);
        assert!(rules[1].selector_text.is_none());
        assert!(rules[1].declaration_text.is_none());
        assert!(rules[1].css_text.contains(".nested"));
        assert_eq!(rules[2].selector_text.as_deref(), Some("#last"));
    }

    #[test]
    fn readback_expands_shorthands_and_preserves_token_boundaries() {
        let view = read_declaration(
            "margin: 1px 2px !important; --Case: 'a;b:c'; width: -1px",
            "margin-left",
            "about:blank",
            false,
        )
        .unwrap();
        assert_eq!(view.value, "2px");
        assert!(view.important);
        assert_eq!(
            view.names,
            [
                "margin-top",
                "margin-right",
                "margin-bottom",
                "margin-left",
                "--Case"
            ]
        );
        let custom = read_declaration(&view.css_text, "--Case", "about:blank", false).unwrap();
        assert!(custom.value.contains("a;b:c"));
        assert_eq!(
            read_declaration(&view.css_text, "--case", "about:blank", false)
                .unwrap()
                .value,
            ""
        );
        assert!(!view.names.iter().any(|name| name == "width"));
    }

    #[test]
    fn invalid_and_absent_mutations_preserve_attribute() {
        for (property, value) in [("width", "-100px"), ("width", ""), ("unknown", "1px")] {
            let result =
                mutate_declaration("top: 0", property, value, "", "about:blank", false).unwrap();
            assert_eq!(result.css_text, None);
        }
    }

    #[test]
    fn shorthand_removal_and_custom_property_case_use_stylo() {
        let removed = mutate_declaration(
            "margin: 1px 2px; color: red",
            "margin",
            "",
            "invalid",
            "about:blank",
            false,
        )
        .unwrap();
        assert_eq!(removed.previous_value, "1px 2px");
        assert_eq!(removed.css_text.as_deref(), Some("color: red;"));
        let changed = mutate_declaration(
            "--Name: one; --name: two",
            "--Name",
            "three",
            "important",
            "about:blank",
            false,
        )
        .unwrap();
        assert_eq!(changed.previous_value, "one");
        let text = changed.css_text.unwrap();
        assert!(text.contains("--Name: three !important;"));
        assert!(text.contains("--name: two;"));
    }
}
