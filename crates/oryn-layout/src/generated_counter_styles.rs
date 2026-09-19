//! Counter representations from Stylo-validated descriptors. Generated text
//! uses counter()/counters() semantics (without marker prefix or suffix).
use super::*;
use style::counter_style::{CounterBound, Descriptors, Symbol, System};
use unicode_segmentation::UnicodeSegmentation;
#[derive(Default)]
pub(super) struct Registry {
    parent: Option<Arc<Registry>>,
    rules: BTreeMap<String, Descriptors>,
    layers: Layers,
    priorities: BTreeMap<String, Vec<u32>>,
}
#[derive(Default)]
struct Layers {
    named: BTreeMap<(Vec<u32>, String), u32>,
    next: BTreeMap<Vec<u32>, u32>,
}
impl Layers {
    fn child(&mut self, parent: &[u32], name: Option<String>) -> Vec<u32> {
        let key = name.as_ref().map(|name| (parent.to_vec(), name.clone()));
        let ordinal = if let Some(index) = key.as_ref().and_then(|key| self.named.get(key)) {
            *index
        } else {
            let next = self.next.entry(parent.to_vec()).or_default();
            let index = *next;
            *next = next.saturating_add(1);
            if let Some(key) = key {
                self.named.insert(key, index);
            }
            index
        };
        let mut result = parent.to_vec();
        result.push(ordinal);
        result
    }
    fn named(
        &mut self,
        parent: &[u32],
        name: &style::stylesheets::layer_rule::LayerName,
    ) -> Vec<u32> {
        let mut path = parent.to_vec();
        for name in name.layer_names() {
            path = self.child(&path, Some(name.0.to_string()));
        }
        path
    }
}
fn symbol(value: &Symbol) -> String {
    match value {
        Symbol::String(value) => value.to_string(),
        Symbol::Ident(value) => value.0.to_string(),
    }
}
impl Registry {
    pub(super) fn read(input: &LayoutInput, print: bool) -> Result<Self, LayoutError> {
        Self::read_tree(input, print, None)
    }
    pub(super) fn read_for_scope(
        input: &LayoutInput,
        print: bool,
        scope: Option<u64>,
    ) -> Result<Arc<Self>, LayoutError> {
        let mut chain = Vec::new();
        let mut current = scope;
        let mut visited = BTreeSet::new();
        while let Some(root) = current {
            if !visited.insert(root) || chain.len() >= 128 {
                return Err(LayoutError::StyleTraversal(
                    "counter style scope ancestry exceeds native bound".into(),
                ));
            }
            chain.push(root);
            let tree =
                input.shadow_trees.roots.get(&root).ok_or_else(|| {
                    LayoutError::StyleTraversal("unknown counter style scope".into())
                })?;
            current = input.shadow_trees.node_scopes.get(&tree.host).copied();
        }
        let mut registry = Arc::new(Self::read(input, print)?);
        for root in chain.into_iter().rev() {
            let mut child = Self::read_tree(input, print, Some(root))?;
            child.parent = Some(registry);
            registry = Arc::new(child);
        }
        Ok(registry)
    }
    fn read_tree(
        input: &LayoutInput,
        print: bool,
        scope: Option<u64>,
    ) -> Result<Self, LayoutError> {
        use style::{
            context::QuirksMode,
            media_queries::{MediaList, MediaType},
            servo_arc::Arc,
            shared_lock::SharedRwLock,
            stylesheets::{
                AllowImportRules, Origin, Stylesheet, StylesheetInDocument, UrlExtraData,
            },
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
            media_type: Some(if print {
                MediaType::print()
            } else {
                MediaType::screen()
            }),
            style_threading: blitz_dom::StyleThreading::Sequential,
            ..Default::default()
        });
        let device = document.stylist_device();
        let mut registry = Self::default();
        for (index, source) in input.stylesheets.iter().enumerate() {
            if input.shadow_trees.stylesheet_scopes.get(&index).copied() != scope {
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
                .unwrap_or(&input.document_base_url);
            let Ok(base) = url::Url::parse(base) else {
                continue;
            };
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
                &mut registry.rules,
                &mut registry.layers,
                &mut registry.priorities,
                &[],
            )?;
        }
        Ok(registry)
    }
    fn resolved(&self, name: &str, visited: &mut BTreeSet<String>) -> Option<Descriptors> {
        if visited.len() >= 32 || !visited.insert(name.into()) {
            return None;
        }
        if !self.rules.contains_key(name) {
            if let Some(parent) = &self.parent {
                visited.remove(name);
                return parent.resolved(name, visited);
            }
        }
        let mut value = self
            .rules
            .get(name)
            .cloned()
            .or_else(|| builtin(name))
            .or_else(|| anonymous(name))?;
        if let Some(System::Extends(base)) = &value.system {
            let parent = self.resolved(base.0.as_ref(), visited)?;
            value.system = parent.system;
            macro_rules! inherit{($($field:ident),*)=>{$(if value.$field.is_none(){value.$field=parent.$field;})*};}
            inherit!(
                negative,
                prefix,
                suffix,
                range,
                pad,
                fallback,
                symbols,
                additive_symbols,
                speak_as
            );
        }
        Some(value)
    }
    fn special_system(&self, name: &str) -> Option<String> {
        self.special_at(name, &mut BTreeSet::new())
    }
    fn special_at(&self, name: &str, visited: &mut BTreeSet<String>) -> Option<String> {
        if !self.rules.contains_key(name) {
            if let Some(parent) = &self.parent {
                return parent.special_at(name, visited);
            }
        }
        if visited.len() >= 32 || !visited.insert(name.into()) {
            return None;
        }
        if let Some(rule) = self.rules.get(name) {
            return match &rule.system {
                Some(System::Extends(parent)) => self.special_at(parent.0.as_ref(), visited),
                _ => None,
            };
        }
        matches!(
            name,
            "ethiopic-numeric"
                | "simp-chinese-informal"
                | "simp-chinese-formal"
                | "trad-chinese-informal"
                | "trad-chinese-formal"
                | "cjk-ideographic"
        )
        .then(|| name.to_owned())
    }
    fn fallback_owner<'a>(&'a self, name: &str, visited: &mut BTreeSet<String>) -> &'a Self {
        if !self.rules.contains_key(name) {
            if let Some(parent) = &self.parent {
                return parent.fallback_owner(name, visited);
            }
        }
        if visited.len() >= 32 || !visited.insert(name.into()) {
            return self;
        }
        let Some(rule) = self.rules.get(name) else {
            return self;
        };
        if rule.fallback.is_none() {
            if let Some(System::Extends(parent)) = &rule.system {
                return self.fallback_owner(parent.0.as_ref(), visited);
            }
        }
        self
    }
    pub(super) fn marker(&self, value: i64, name: &str) -> String {
        if !self.rules.contains_key(name) {
            if let Some(parent) = &self.parent {
                return parent.marker(value, name);
            }
        }
        if name == "none" {
            return String::new();
        }
        let mut input = cssparser::ParserInput::new(name);
        let mut parser = cssparser::Parser::new(&mut input);
        if let Ok(text) = parser.expect_string_cloned() {
            return text.to_string();
        }
        let rule = self.resolved(name, &mut BTreeSet::new());
        let prefix = rule
            .as_ref()
            .and_then(|rule| rule.prefix.as_ref())
            .map(symbol)
            .unwrap_or_default();
        let suffix = rule
            .as_ref()
            .and_then(|rule| rule.suffix.as_ref())
            .map(symbol)
            .unwrap_or_else(|| {
                if matches!(
                    name,
                    "disc" | "circle" | "square" | "disclosure-open" | "disclosure-closed"
                ) || name.starts_with("symbols(")
                {
                    " ".into()
                } else {
                    ". ".into()
                }
            });
        format!("{prefix}{}{suffix}", self.format(value, name))
    }
    pub(super) fn format(&self, value: i64, name: &str) -> String {
        self.format_inner(value, name, &mut BTreeSet::new())
    }
    fn format_inner(&self, value: i64, name: &str, fallbacks: &mut BTreeSet<String>) -> String {
        if !self.rules.contains_key(name) {
            if let Some(parent) = &self.parent {
                return parent.format_inner(value, name, fallbacks);
            }
        }
        if fallbacks.len() >= 32 || !fallbacks.insert(name.into()) {
            return super::generated::format_counter(value, "decimal");
        }
        let Some(rule) = self.resolved(name, &mut BTreeSet::new()) else {
            return super::generated::format_counter(value, name);
        };
        let system = rule.system.as_ref().unwrap_or(&System::Symbolic);
        let fallback_owner = self.fallback_owner(name, &mut BTreeSet::new());
        let fallback = |visited: &mut BTreeSet<String>| {
            fallback_owner.format_inner(
                value,
                rule.fallback
                    .as_ref()
                    .map(|name| name.0.0.as_ref())
                    .unwrap_or("decimal"),
                visited,
            )
        };
        let allowed =
            if let Some(ranges) = rule.range.as_ref().filter(|ranges| !ranges.0.is_empty()) {
                ranges.0.iter().any(|range| {
                    let lower = match &range.start {
                        CounterBound::Infinite => i64::MIN,
                        CounterBound::Integer(number) => {
                            number.resolve().map(i64::from).unwrap_or(i64::MIN)
                        }
                    };
                    let upper = match &range.end {
                        CounterBound::Infinite => i64::MAX,
                        CounterBound::Integer(number) => {
                            number.resolve().map(i64::from).unwrap_or(i64::MAX)
                        }
                    };
                    value >= lower && value <= upper
                })
            } else {
                match system {
                    System::Alphabetic | System::Symbolic => value >= 1,
                    System::Additive => value >= 0,
                    _ => true,
                }
            };
        if !allowed {
            return fallback(fallbacks);
        }
        let symbols = rule
            .symbols
            .as_ref()
            .map(|symbols| symbols.0.iter().map(symbol).collect::<Vec<_>>())
            .unwrap_or_default();
        let magnitude = value.unsigned_abs();
        let mut representation = String::new();
        let append = |target: &mut String, value: &str, count: u64| -> bool {
            if value.is_empty() {
                return true;
            }
            let Some(bytes) = (value.len() as u64).checked_mul(count) else {
                return false;
            };
            if bytes > 1_048_576u64.saturating_sub(target.len() as u64) {
                return false;
            }
            for _ in 0..count {
                target.push_str(value);
            }
            true
        };
        let valid = if let Some(system) = self.special_system(name) {
            if let Some(text) = special_representation(magnitude, &system) {
                representation = text;
                true
            } else {
                false
            }
        } else {
            match system {
                System::Cyclic if !symbols.is_empty() => {
                    representation = symbols
                        [(value as i128 - 1).rem_euclid(symbols.len() as i128) as usize]
                        .clone();
                    true
                }
                System::Fixed { first_symbol_value } if !symbols.is_empty() => {
                    let first = first_symbol_value
                        .as_ref()
                        .and_then(|value| value.resolve())
                        .map(i64::from)
                        .unwrap_or(1);
                    let index = value as i128 - first as i128;
                    if index >= 0 && (index as usize) < symbols.len() {
                        representation = symbols[index as usize].clone();
                        true
                    } else {
                        false
                    }
                }
                System::Numeric if symbols.len() >= 2 => {
                    let mut number = magnitude;
                    let mut digits = Vec::new();
                    loop {
                        digits.push(symbols[(number % symbols.len() as u64) as usize].as_str());
                        number /= symbols.len() as u64;
                        if number == 0 {
                            break;
                        }
                    }
                    for digit in digits.into_iter().rev() {
                        if !append(&mut representation, digit, 1) {
                            return fallback(fallbacks);
                        }
                    }
                    true
                }
                System::Alphabetic if symbols.len() >= 2 && magnitude > 0 => {
                    let mut number = magnitude;
                    let mut digits = Vec::new();
                    while number > 0 {
                        number -= 1;
                        digits.push(symbols[(number % symbols.len() as u64) as usize].as_str());
                        number /= symbols.len() as u64;
                    }
                    for digit in digits.into_iter().rev() {
                        if !append(&mut representation, digit, 1) {
                            return fallback(fallbacks);
                        }
                    }
                    true
                }
                System::Symbolic if !symbols.is_empty() && magnitude > 0 => append(
                    &mut representation,
                    &symbols[((magnitude - 1) % symbols.len() as u64) as usize],
                    (magnitude - 1) / symbols.len() as u64 + 1,
                ),
                System::Additive => {
                    let mut number = magnitude;
                    let mut valid = false;
                    if let Some(symbols) = &rule.additive_symbols {
                        for tuple in symbols.0.iter() {
                            let weight = tuple.weight.resolve().unwrap_or(0).max(0) as u64;
                            if weight == 0 {
                                if magnitude == 0 {
                                    valid = append(&mut representation, &symbol(&tuple.symbol), 1);
                                }
                                continue;
                            }
                            let count = number / weight;
                            if count > 0 {
                                if !append(&mut representation, &symbol(&tuple.symbol), count) {
                                    return fallback(fallbacks);
                                }
                                number %= weight;
                                valid = true;
                            }
                        }
                    }
                    valid && number == 0
                }
                _ => false,
            }
        };
        if !valid || representation.len() > 1_048_576 {
            return fallback(fallbacks);
        }
        let negative = value < 0 && !matches!(system, System::Cyclic | System::Fixed { .. });
        let (before, after) = if negative {
            rule.negative
                .as_ref()
                .map(|value| {
                    (
                        symbol(&value.0),
                        value.1.as_ref().map(symbol).unwrap_or_default(),
                    )
                })
                .unwrap_or_else(|| ("-".into(), String::new()))
        } else {
            (String::new(), String::new())
        };
        if let Some(pad) = &rule.pad {
            let minimum = pad.0.resolve().unwrap_or(0).max(0) as usize;
            let current = representation.graphemes(true).count()
                + before.graphemes(true).count()
                + after.graphemes(true).count();
            let mut padding = String::new();
            if !append(
                &mut padding,
                &symbol(&pad.1),
                minimum.saturating_sub(current) as u64,
            ) {
                return fallback(fallbacks);
            }
            padding.push_str(&representation);
            representation = padding;
        }
        if before
            .len()
            .saturating_add(representation.len())
            .saturating_add(after.len())
            > 1_048_576
        {
            return fallback(fallbacks);
        }
        format!("{before}{representation}{after}")
    }
}
fn collect(
    rules: &[style::stylesheets::CssRule],
    guard: &style::shared_lock::SharedRwLockReadGuard,
    device: &style::device::Device,
    out: &mut BTreeMap<String, Descriptors>,
    layers: &mut Layers,
    priorities: &mut BTreeMap<String, Vec<u32>>,
    path: &[u32],
) -> Result<(), LayoutError> {
    use style::{
        context::QuirksMode,
        stylesheets::{CssRule, CustomMediaEvaluator},
    };
    if path.len() > 64 || layers.named.len() > 4096 || layers.next.len() > 4096 {
        return Err(LayoutError::StyleTraversal(
            "counter layer registry bound exceeded".into(),
        ));
    }
    for rule in rules {
        match rule {
            CssRule::CounterStyle(rule) => {
                let rule = rule.read_with(guard);
                if out.len() >= 4096 && !out.contains_key(rule.name().0.as_ref()) {
                    return Err(LayoutError::StyleTraversal(
                        "counter style registry exceeds 4096 names".into(),
                    ));
                }
                let name = rule.name().0.to_string();
                let mut priority = path.to_vec();
                priority.push(u32::MAX);
                if priorities
                    .get(&name)
                    .is_none_or(|previous| previous <= &priority)
                {
                    out.insert(name.clone(), rule.descriptors().clone());
                    priorities.insert(name, priority);
                }
            }
            CssRule::Media(media) => {
                if media.media_queries.read_with(guard).evaluate(
                    device,
                    QuirksMode::NoQuirks,
                    &mut CustomMediaEvaluator::none(),
                ) {
                    collect(
                        rule.children(guard),
                        guard,
                        device,
                        out,
                        layers,
                        priorities,
                        path,
                    )?;
                }
            }
            CssRule::Supports(supports) if supports.enabled => collect(
                rule.children(guard),
                guard,
                device,
                out,
                layers,
                priorities,
                path,
            )?,
            CssRule::LayerBlock(block) => {
                let nested = if let Some(name) = &block.name {
                    layers.named(path, name)
                } else {
                    layers.child(path, None)
                };
                collect(
                    rule.children(guard),
                    guard,
                    device,
                    out,
                    layers,
                    priorities,
                    &nested,
                )?;
            }
            CssRule::LayerStatement(statement) => {
                for name in &statement.names {
                    layers.named(path, name);
                }
            }
            // Name-defining rules are not affected by the enclosing selector scope.
            CssRule::Scope(_) => collect(
                rule.children(guard),
                guard,
                device,
                out,
                layers,
                priorities,
                path,
            )?,
            _ => {}
        }
    }
    Ok(())
}

// Built-ins participate in extends just like author descriptors; formatting
// them only at the last fallback step would lose inherited systems and ranges.
fn builtin(name: &str) -> Option<Descriptors> {
    use style::counter_style::{
        AdditiveSymbols, AdditiveTuple, CounterRange, CounterRanges, Pad, Symbols,
    };
    use style::values::specified::Integer;
    let string = |value: String| Symbol::String(value.into());
    let mut rule = Descriptors::default();
    if matches!(
        name,
        "ethiopic-numeric"
            | "simp-chinese-informal"
            | "simp-chinese-formal"
            | "trad-chinese-informal"
            | "trad-chinese-formal"
            | "cjk-ideographic"
    ) {
        rule.system = Some(System::Symbolic);
        let ethiopic = name == "ethiopic-numeric";
        rule.range = Some(CounterRanges(
            vec![CounterRange {
                start: CounterBound::Integer(Integer::new(if ethiopic { 1 } else { -9999 })),
                end: if ethiopic {
                    CounterBound::Infinite
                } else {
                    CounterBound::Integer(Integer::new(9999))
                },
            }]
            .into(),
        ));
        rule.suffix = Some(string(if ethiopic { "/ " } else { "、" }.into()));
        if !ethiopic {
            rule.negative = Some(style::counter_style::Negative(
                string(
                    if name.starts_with("simp-") {
                        "负"
                    } else {
                        "負"
                    }
                    .into(),
                ),
                None,
            ));
            rule.fallback = Some(style::counter_style::Fallback(style::values::CustomIdent(
                style::Atom::from("cjk-decimal"),
            )));
        }
        return Some(rule);
    }
    let numeric = match name {
        "decimal" | "decimal-leading-zero" => Some('0'),
        "arabic-indic" => Some('٠'),
        "bengali" => Some('০'),
        "cambodian" | "khmer" => Some('០'),
        "devanagari" => Some('०'),
        "gujarati" => Some('૦'),
        "gurmukhi" => Some('੦'),
        "kannada" => Some('೦'),
        "lao" => Some('໐'),
        "malayalam" => Some('൦'),
        "mongolian" => Some('᠐'),
        "myanmar" => Some('၀'),
        "oriya" => Some('୦'),
        "persian" => Some('۰'),
        "tamil" => Some('௦'),
        "telugu" => Some('౦'),
        "thai" => Some('๐'),
        "tibetan" => Some('༠'),
        _ => None,
    };
    if let Some(zero) = numeric {
        rule.system = Some(System::Numeric);
        rule.symbols = Some(Symbols(style::ArcSlice::from_iter((0..10).map(|offset| {
            string(char::from_u32(zero as u32 + offset).unwrap().to_string())
        }))));
        if name == "decimal-leading-zero" {
            rule.pad = Some(Pad(Integer::new(2), string("0".into())));
        }
        return Some(rule);
    }
    if name == "cjk-decimal" {
        rule.range = Some(CounterRanges(
            vec![CounterRange {
                start: CounterBound::Integer(Integer::new(0)),
                end: CounterBound::Infinite,
            }]
            .into(),
        ));
        rule.system = Some(System::Numeric);
        rule.symbols = Some(Symbols(style::ArcSlice::from_iter(
            "〇一二三四五六七八九"
                .chars()
                .map(|ch| string(ch.to_string()))
                .collect::<Vec<_>>()
                .into_iter(),
        )));
        rule.suffix = Some(string("、".into()));
        return Some(rule);
    }
    if matches!(name, "cjk-earthly-branch" | "cjk-heavenly-stem") {
        let alphabet = if name == "cjk-earthly-branch" {
            "子丑寅卯辰巳午未申酉戌亥"
        } else {
            "甲乙丙丁戊己庚辛壬癸"
        };
        rule.fallback = Some(style::counter_style::Fallback(style::values::CustomIdent(
            style::Atom::from("cjk-decimal"),
        )));
        rule.system = Some(System::Fixed {
            first_symbol_value: None,
        });
        rule.symbols = Some(Symbols(style::ArcSlice::from_iter(
            alphabet
                .chars()
                .map(|ch| string(ch.to_string()))
                .collect::<Vec<_>>()
                .into_iter(),
        )));
        rule.suffix = Some(string("、".into()));
        return Some(rule);
    }
    if matches!(name, "armenian" | "upper-armenian" | "lower-armenian") {
        rule.system = Some(System::Additive);
        rule.range = Some(CounterRanges(
            vec![CounterRange {
                start: CounterBound::Integer(Integer::new(1)),
                end: CounterBound::Integer(Integer::new(9999)),
            }]
            .into(),
        ));
        let letters = "ԱԲԳԴԵԶԷԸԹԺԻԼԽԾԿՀՁՂՃՄՅՆՇՈՉՊՋՌՍՎՏՐՑՒՓՔ";
        let mut symbols = letters
            .chars()
            .enumerate()
            .map(|(index, ch)| AdditiveTuple {
                weight: Integer::new(((index % 9 + 1) as i32) * 10i32.pow((index / 9) as u32)),
                symbol: string(if name == "lower-armenian" {
                    ch.to_lowercase().collect()
                } else {
                    ch.to_string()
                }),
            })
            .collect::<Vec<_>>();
        symbols.reverse();
        rule.additive_symbols = Some(AdditiveSymbols(symbols.into()));
        return Some(rule);
    }
    // Normative limited additive systems from CSS Counter Styles 3,
    // https://drafts.csswg.org/css-counter-styles/#limited-japanese and Korean.
    if matches!(
        name,
        "japanese-informal"
            | "japanese-formal"
            | "korean-hangul-formal"
            | "korean-hanja-informal"
            | "korean-hanja-formal"
    ) {
        let (digits, units, omit_one, negative, suffix) = match name {
            "japanese-informal" => (
                "〇一二三四五六七八九",
                ["", "十", "百", "千"],
                true,
                "マイナス",
                "、",
            ),
            "japanese-formal" => (
                "零壱弐参四伍六七八九",
                ["", "拾", "百", "阡"],
                false,
                "マイナス",
                "、",
            ),
            "korean-hangul-formal" => (
                "영일이삼사오육칠팔구",
                ["", "십", "백", "천"],
                false,
                "마이너스 ",
                ", ",
            ),
            "korean-hanja-formal" => (
                "零壹貳參四五六七八九",
                ["", "拾", "百", "仟"],
                false,
                "마이너스 ",
                ", ",
            ),
            _ => (
                "零一二三四五六七八九",
                ["", "十", "百", "千"],
                true,
                "마이너스 ",
                ", ",
            ),
        };
        let digits = digits.chars().collect::<Vec<_>>();
        let mut symbols = Vec::new();
        for place in (0..4usize).rev() {
            for digit in (1..=9usize).rev() {
                symbols.push(AdditiveTuple {
                    weight: Integer::new(digit as i32 * 10i32.pow(place as u32)),
                    symbol: string(format!(
                        "{}{}",
                        if omit_one && digit == 1 && place > 0 {
                            String::new()
                        } else {
                            digits[digit].to_string()
                        },
                        units[place]
                    )),
                });
            }
        }
        symbols.push(AdditiveTuple {
            weight: Integer::new(0),
            symbol: string(digits[0].to_string()),
        });
        rule.system = Some(System::Additive);
        rule.range = Some(CounterRanges(
            vec![CounterRange {
                start: CounterBound::Integer(Integer::new(-9999)),
                end: CounterBound::Integer(Integer::new(9999)),
            }]
            .into(),
        ));
        rule.additive_symbols = Some(AdditiveSymbols(symbols.into()));
        rule.negative = Some(style::counter_style::Negative(
            string(negative.into()),
            None,
        ));
        rule.suffix = Some(string(suffix.into()));
        rule.fallback = Some(style::counter_style::Fallback(style::values::CustomIdent(
            style::Atom::from("cjk-decimal"),
        )));
        return Some(rule);
    }
    if matches!(name, "georgian" | "hebrew") {
        let (maximum, weights, symbols): (i32, Vec<i32>, Vec<&str>) = if name == "georgian" {
            (
                19999,
                vec![
                    10000, 9000, 8000, 7000, 6000, 5000, 4000, 3000, 2000, 1000, 900, 800, 700,
                    600, 500, 400, 300, 200, 100, 90, 80, 70, 60, 50, 40, 30, 20, 10, 9, 8, 7, 6,
                    5, 4, 3, 2, 1,
                ],
                vec![
                    "ჵ", "ჰ", "ჯ", "ჴ", "ხ", "ჭ", "წ", "ძ", "ც", "ჩ", "შ", "ყ", "ღ", "ქ", "ფ", "ჳ",
                    "ტ", "ს", "რ", "ჟ", "პ", "ო", "ჲ", "ნ", "მ", "ლ", "კ", "ი", "თ", "ჱ", "ზ", "ვ",
                    "ე", "დ", "გ", "ბ", "ა",
                ],
            )
        } else {
            (
                10999,
                vec![
                    10000, 9000, 8000, 7000, 6000, 5000, 4000, 3000, 2000, 1000, 400, 300, 200,
                    100, 90, 80, 70, 60, 50, 40, 30, 20, 19, 18, 17, 16, 15, 10, 9, 8, 7, 6, 5, 4,
                    3, 2, 1,
                ],
                vec![
                    "י׳", "ט׳", "ח׳", "ז׳", "ו׳", "ה׳", "ד׳", "ג׳", "ב׳", "א׳", "ת", "ש", "ר", "ק",
                    "צ", "פ", "ע", "ס", "נ", "מ", "ל", "כ", "יט", "יח", "יז", "טז", "טו", "י", "ט",
                    "ח", "ז", "ו", "ה", "ד", "ג", "ב", "א",
                ],
            )
        };
        rule.system = Some(System::Additive);
        rule.range = Some(CounterRanges(
            vec![CounterRange {
                start: CounterBound::Integer(Integer::new(1)),
                end: CounterBound::Integer(Integer::new(maximum)),
            }]
            .into(),
        ));
        rule.additive_symbols = Some(AdditiveSymbols(
            weights
                .into_iter()
                .zip(symbols)
                .map(|(weight, text)| AdditiveTuple {
                    weight: Integer::new(weight),
                    symbol: string(text.into()),
                })
                .collect::<Vec<_>>()
                .into(),
        ));
        return Some(rule);
    }
    let alphabet = match name {
        "lower-alpha" | "lower-latin" => Some("abcdefghijklmnopqrstuvwxyz"),
        "upper-alpha" | "upper-latin" => Some("ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
        "lower-greek" => Some("αβγδεζηθικλμνξοπρστυφχψω"),
        "hiragana" => Some(
            "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわゐゑをん",
        ),
        "katakana" => Some(
            "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヰヱヲン",
        ),
        "hiragana-iroha" => Some(
            "いろはにほへとちりぬるをわかよたれそつねならむうゐのおくやまけふこえてあさきゆめみしゑひもせす",
        ),
        "katakana-iroha" => Some(
            "イロハニホヘトチリヌルヲワカヨタレソツネナラムウヰノオクヤマケフコエテアサキユメミシヱヒモセス",
        ),
        _ => None,
    };
    if let Some(alphabet) = alphabet {
        if name.starts_with("hiragana") || name.starts_with("katakana") {
            rule.suffix = Some(string("、".into()));
        }
        rule.system = Some(System::Alphabetic);
        rule.symbols = Some(Symbols(style::ArcSlice::from_iter(
            alphabet
                .chars()
                .map(|ch| string(ch.to_string()))
                .collect::<Vec<_>>()
                .into_iter(),
        )));
        return Some(rule);
    }
    if matches!(name, "lower-roman" | "upper-roman") {
        rule.system = Some(System::Additive);
        rule.range = Some(CounterRanges(
            vec![CounterRange {
                start: CounterBound::Integer(Integer::new(1)),
                end: CounterBound::Integer(Integer::new(3999)),
            }]
            .into(),
        ));
        rule.additive_symbols = Some(AdditiveSymbols(
            [
                (1000, "M"),
                (900, "CM"),
                (500, "D"),
                (400, "CD"),
                (100, "C"),
                (90, "XC"),
                (50, "L"),
                (40, "XL"),
                (10, "X"),
                (9, "IX"),
                (5, "V"),
                (4, "IV"),
                (1, "I"),
            ]
            .into_iter()
            .map(|(weight, text)| AdditiveTuple {
                weight: Integer::new(weight),
                symbol: string(if name == "lower-roman" {
                    text.to_ascii_lowercase()
                } else {
                    text.into()
                }),
            })
            .collect::<Vec<_>>()
            .into(),
        ));
        return Some(rule);
    }
    let bullet = match name {
        "disc" => "•",
        "circle" => "◦",
        "square" => "▪",
        "disclosure-open" => "▾",
        "disclosure-closed" => "▸",
        _ => return None,
    };
    rule.system = Some(System::Cyclic);
    rule.suffix = Some(string(" ".into()));
    rule.symbols = Some(Symbols(style::ArcSlice::from_iter(
        [string(bullet.into())].into_iter(),
    )));
    Some(rule)
}

fn anonymous(source: &str) -> Option<Descriptors> {
    let mut input = cssparser::ParserInput::new(source);
    let mut parser = cssparser::Parser::new(&mut input);
    parser.expect_function_matching("symbols").ok()?;
    let rule = parser
        .parse_nested_block(
            |parser| -> Result<Descriptors, cssparser::ParseError<'_, ()>> {
                let system = match parser
                    .try_parse(|p| p.expect_ident_cloned())
                    .ok()
                    .as_deref()
                {
                    None | Some("symbolic") => System::Symbolic,
                    Some("cyclic") => System::Cyclic,
                    Some("numeric") => System::Numeric,
                    Some("alphabetic") => System::Alphabetic,
                    Some("fixed") => System::Fixed {
                        first_symbol_value: None,
                    },
                    _ => return Err(parser.new_custom_error(())),
                };
                let mut symbols = Vec::new();
                while !parser.is_exhausted() {
                    if symbols.len() >= 4096 {
                        return Err(parser.new_custom_error(()));
                    }
                    symbols.push(Symbol::String(
                        parser.expect_string_cloned()?.to_string().into(),
                    ));
                }
                if symbols.is_empty()
                    || (matches!(system, System::Numeric | System::Alphabetic) && symbols.len() < 2)
                {
                    return Err(parser.new_custom_error(()));
                }
                let mut rule = Descriptors::default();
                rule.system = Some(system);
                rule.symbols = Some(style::counter_style::Symbols(style::ArcSlice::from_iter(
                    symbols.into_iter(),
                )));
                Ok(rule)
            },
        )
        .ok()?;
    parser.expect_exhausted().ok()?;
    Some(rule)
}

/// Normative non-additive algorithms, CSS Counter Styles 3 limited-chinese and
/// ethiopic-numeric. The optional extended CJK range is deliberately not selected.
fn special_representation(value: u64, name: &str) -> Option<String> {
    if name == "ethiopic-numeric" {
        if value == 0 {
            return None;
        }
        if value == 1 {
            return Some("፩".into());
        }
        let mut groups = Vec::new();
        let mut remaining = value;
        while remaining > 0 {
            groups.push(remaining % 100);
            remaining /= 100;
        }
        let mut result = String::new();
        for index in (0..groups.len()).rev() {
            let group = groups[index];
            if group != 0 && !(group == 1 && (index == groups.len() - 1 || index % 2 == 1)) {
                if group >= 10 {
                    result.push(char::from_u32(0x1371 + (group / 10) as u32).unwrap());
                }
                if group % 10 > 0 {
                    result.push(char::from_u32(0x1368 + (group % 10) as u32).unwrap());
                }
            }
            if index % 2 == 1 && group != 0 {
                result.push('፻');
            } else if index > 0 && index % 2 == 0 {
                result.push('፼');
            }
        }
        return Some(result);
    }
    if value > 9999 {
        return None;
    }
    let informal = name.ends_with("informal") || name == "cjk-ideographic";
    let digits = if informal {
        "零一二三四五六七八九"
    } else if name.starts_with("simp-") {
        "零壹贰叁肆伍陆柒捌玖"
    } else {
        "零壹貳參肆伍陸柒捌玖"
    }
    .chars()
    .collect::<Vec<_>>();
    let units = if informal {
        ["", "十", "百", "千"]
    } else {
        ["", "拾", "佰", "仟"]
    };
    if value == 0 {
        return Some(digits[0].to_string());
    }
    let mut result = String::new();
    let mut pending_zero = false;
    for place in (0..4usize).rev() {
        let digit = (value / 10u64.pow(place as u32) % 10) as usize;
        if digit == 0 {
            if !result.is_empty() {
                pending_zero = true;
            }
            continue;
        }
        if pending_zero {
            result.push(digits[0]);
            pending_zero = false;
        }
        if !(informal && (10..=19).contains(&value) && place == 1) {
            result.push(digits[digit]);
        }
        result.push_str(units[place]);
    }
    Some(result)
}
