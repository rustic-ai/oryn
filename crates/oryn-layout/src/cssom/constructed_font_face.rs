// Included in native_import_graph. Parser-internal allocation remains Stylo's
// opaque allocation boundary; this API never builds a synthetic stylesheet.
pub struct NativeConstructedFontFace {
    descriptors: LeaseArc<Mutex<style::font_face::Descriptors>>,
    base: UrlExtraData,
    revision: u64,
    epoch: LeaseArc<std::sync::atomic::AtomicU64>,
    pub initial_error: Option<String>,
    _base_allocation: LeaseArc<dyn Send + Sync>,
    _diagnostic_allocation: Option<LeaseArc<dyn Send + Sync>>,
    descriptor_allocations:
        [Option<LeaseArc<dyn Send + Sync>>; style::font_face::DescriptorId::COUNT],
}
pub struct NativeConstructedFontDemand {
    pub identity: usize,
    pub revision: u64,
    pub family: String,
    pub descriptors: String,
    pub sources: Vec<NativeFontSource>,
    pub unicode_ranges: Vec<(u32, u32)>,
    pub base_url: String,
    _allocation: LeaseArc<dyn Send + Sync>,
}
impl NativeConstructedFontFace {
    pub fn identity(&self) -> usize {
        LeaseArc::as_ptr(&self.descriptors) as usize
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn demand(
        &self,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeConstructedFontDemand, String> {
        self.edit_context().demand(admission)
    }
    pub fn parse(
        family: &str,
        source: &str,
        pairs: &[(&str, &str)],
        base: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<Self, String> {
        Self::parse_descriptors(family, Some(source), pairs, base, admission)
    }
    /// Binary source bytes stay in the native adapter and decoder custody;
    /// they are not represented by a synthetic CSS src descriptor.
    pub fn parse_binary(
        family: &str,
        pairs: &[(&str, &str)],
        base: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<Self, String> {
        Self::parse_descriptors(family, None, pairs, base, admission)
    }
    fn parse_descriptors(
        family: &str,
        source: Option<&str>,
        pairs: &[(&str, &str)],
        base: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<Self, String> {
        use style::font_face::{DescriptorId, Descriptors};
        let input_bytes = family
            .len()
            .checked_add(source.map_or(0, str::len))
            .and_then(|n| n.checked_add(base.len()))
            .and_then(|n| {
                pairs
                    .iter()
                    .try_fold(n, |n, (a, b)| n.checked_add(a.len())?.checked_add(b.len()))
            })
            .ok_or("constructed font input overflow")?;
        let allocation = admission.reserve(
            input_bytes
                .checked_add(std::mem::size_of::<Self>())
                .and_then(|n| n.checked_add(std::mem::size_of::<Mutex<Descriptors>>()))
                .and_then(|n| n.checked_add(4 * std::mem::size_of::<usize>()))
                .and_then(|n| n.checked_add(std::mem::size_of::<std::sync::atomic::AtomicU64>()))
                .ok_or("constructed font metadata overflow")?,
        )?;
        let diagnostic_allocation = admission.reserve(27)?;
        admission.checkpoint()?;
        let base = UrlExtraData::from(url::Url::parse(base).map_err(|e| e.to_string())?);
        let context = font_constructed_context(&base);
        let mut descriptors = Descriptors::default();
        let mut family_input = cssparser::ParserInput::new(family);
        if descriptors
            .set(
                DescriptorId::FontFamily,
                &context,
                &mut cssparser::Parser::new(&mut family_input),
            )
            .is_err()
        {
            descriptors.font_family = Some(style::values::computed::font::FamilyName {
                name: style::Atom::from(family),
                syntax: style::values::computed::font::FontFamilyNameSyntax::Quoted,
            });
        }
        let mut initial_error = None;
        if let Some(source) = source {
            let mut source_input = cssparser::ParserInput::new(source);
            if descriptors
                .set(
                    DescriptorId::Src,
                    &context,
                    &mut cssparser::Parser::new(&mut source_input),
                )
                .is_err()
            {
                initial_error = Some("invalid FontFace source".into());
            }
        }
        for &(name, value) in pairs {
            let id = match DescriptorId::from_ident(name) {
                Ok(id) => id,
                Err(_) => {
                    if initial_error.is_none() {
                        initial_error = Some("unknown FontFace descriptor".into());
                    }
                    continue;
                }
            };
            let mut input = cssparser::ParserInput::new(value);
            if let Err(_error) =
                descriptors.set(id, &context, &mut cssparser::Parser::new(&mut input))
            {
                if initial_error.is_none() {
                    initial_error = Some("invalid FontFace descriptor".into());
                }
            }
        }
        Ok(Self {
            descriptors: LeaseArc::new(Mutex::new(descriptors)),
            base,
            revision: 0,
            epoch: LeaseArc::new(std::sync::atomic::AtomicU64::new(0)),
            initial_error,
            _base_allocation: allocation,
            _diagnostic_allocation: Some(diagnostic_allocation),
            descriptor_allocations: std::array::from_fn(|_| None),
        })
    }
    /// Capture under caller's readonly borrow, then drop that borrow before prepare.
    pub fn edit_context(&self) -> NativeConstructedEditContext {
        NativeConstructedEditContext {
            identity: self.descriptors.clone(),
            base: self.base.clone(),
            revision: self.revision,
            epoch: self.epoch.clone(),
        }
    }
    pub fn commit_descriptor(
        &mut self,
        edit: PreparedConstructedDescriptor,
    ) -> Result<NativeConstructedCommit, PreparedConstructedDescriptor> {
        if !LeaseArc::ptr_eq(&self.descriptors, &edit.identity)
            || self.revision != edit.expected_revision
        {
            return Err(edit);
        }
        let original_identity = self.descriptors.clone();
        let mut original = match original_identity.lock() {
            Ok(guard) => guard,
            Err(_) => return Err(edit),
        };
        let PreparedConstructedDescriptor {
            identity: _,
            id,
            slot,
            expected_revision: _,
            next_revision,
            mut staged,
            allocation,
        } = edit;
        if original.same_one(id, &staged) {
            drop(original);
            return Ok(NativeConstructedCommit {
                changed: false,
                obsolete_descriptor: Some(staged),
                obsolete_lease: Some(allocation),
            });
        }
        original.swap_one(id, &mut staged);
        drop(original);
        let previous = self.descriptor_allocations[slot].replace(allocation);
        self.revision = next_revision;
        self.epoch
            .store(next_revision, std::sync::atomic::Ordering::Release);
        Ok(NativeConstructedCommit {
            changed: true,
            obsolete_descriptor: Some(staged),
            obsolete_lease: previous,
        })
    }
    pub fn set_descriptor(
        &mut self,
        name: &str,
        value: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<bool, String> {
        let prepared = self.edit_context().prepare(name, value, admission)?;
        let receipt = self
            .commit_descriptor(prepared)
            .map_err(|_| "constructed descriptor edit stale".to_owned())?;
        let changed = receipt.changed;
        drop(receipt);
        Ok(changed)
    }
    pub fn descriptor(
        &self,
        name: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeConstructedDescriptor, String> {
        self.edit_context().descriptor(name, admission)
    }
    pub fn axes(&self) -> Result<NativeFontAxes, String> {
        self.edit_context().axes()
    }
}
impl NativeRule {
    pub fn font_axes(&self) -> Result<NativeFontAxes, String> {
        let CssRule::FontFace(rule) = &self.rule else {
            return Err("native rule is not FontFace".into());
        };
        let shared_lock = self.owner.shared_lock();
        let guard = shared_lock.read();
        font_descriptor_axes(&rule.read_with(&guard).descriptors)
    }
}
pub struct NativeConstructedDescriptor {
    pub value: String,
    _allocation: LeaseArc<dyn Send + Sync>,
}
fn font_descriptor_axes(d: &style::font_face::Descriptors) -> Result<NativeFontAxes, String> {
    use style::values::computed::font::FontStyle;
    let weight = match &d.font_weight {
        None => (400., 400.),
        Some(r) => {
            let r = r.compute().ok_or("unresolved font weight")?;
            (r.0.value(), r.1.value())
        }
    };
    let width = match &d.font_stretch {
        None => (100., 100.),
        Some(r) => {
            let r = r.compute().ok_or("unresolved font stretch")?;
            (r.0.to_percentage().0 * 100., r.1.to_percentage().0 * 100.)
        }
    };
    let style = match &d.font_style {
        None => NativeFontStyle::Normal,
        Some(r) => {
            let r = r.compute().ok_or("unresolved font style")?;
            if r.0 == FontStyle::ITALIC {
                NativeFontStyle::Italic
            } else if r.0 == FontStyle::NORMAL && r.1 == FontStyle::NORMAL {
                NativeFontStyle::Normal
            } else {
                NativeFontStyle::Oblique(r.0.oblique_degrees(), r.1.oblique_degrees())
            }
        }
    };
    Ok(NativeFontAxes {
        weight,
        width,
        style,
    })
}
fn font_constructed_context(base: &UrlExtraData) -> style::parser::ParserContext<'_> {
    style::parser::ParserContext::new(
        Origin::Author,
        base,
        Some(style::stylesheets::CssRuleType::FontFace),
        style_traits::ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        Default::default(),
        None,
        None,
        Default::default(),
    )
}
pub struct NativeFontShorthand {
    pub declarations: style::properties::SourcePropertyDeclaration,
    _allocation: LeaseArc<dyn Send + Sync>,
}
pub struct NativeCanvasFontSerialization {
    pub value: String,
    _allocation: LeaseArc<dyn Send + Sync>,
}
impl NativeCanvasFontSerialization {
    pub fn into_parts(self) -> (String, LeaseArc<dyn Send + Sync>) { (self.value, self._allocation) }
}
/// Canonical native CSS font shorthand serialization. Canvas ignores the
/// shorthand's line-height component; normalize it before native serialization.
pub fn serialize_native_canvas_font(
    computed: &style::properties::ComputedValues, admission: &dyn ImportAdmission,
) -> Result<NativeCanvasFontSerialization, String> {
    use style::properties::{ShorthandId, LonghandId, PropertyDeclaration};
    use style_traits::ToCss;
    admission.checkpoint()?;
    let count = ShorthandId::Font.longhands().count();
    let _scratch = admission.reserve(count.checked_mul(std::mem::size_of::<PropertyDeclaration>()).ok_or("canvas font serialization overflow")?)?;
    let mut declarations = Vec::new();
    declarations.try_reserve_exact(count).map_err(|e| e.to_string())?;
    if declarations.capacity() > count { return Err("canvas font serialization exceeded admission".into()); }
    for id in ShorthandId::Font.longhands() {
        admission.checkpoint()?;
        declarations.push(if id == LonghandId::LineHeight {
            PropertyDeclaration::LineHeight(style::properties::longhands::line_height::get_initial_specified_value())
        } else { computed.computed_or_resolved_declaration(id, None) });
    }
    let shorthand = style::properties::shorthands::font::LonghandsToSerialize::from_iter(declarations.iter()).map_err(|_| "native canvas font shorthand incomplete")?;
    let mut count = FontDescriptorCount::default();
    shorthand.to_css(&mut style_traits::CssWriter::new(&mut count)).map_err(|e| e.to_string())?;
    if count.bytes == 0 { return Err("native canvas font is not serializable".into()); }
    let allocation = admission.reserve(count.bytes.checked_add(std::mem::size_of::<NativeCanvasFontSerialization>()).ok_or("canvas font output overflow")?)?;
    admission.checkpoint()?;
    let mut value = String::new();
    value.try_reserve_exact(count.bytes).map_err(|e| e.to_string())?;
    if value.capacity() > count.bytes { return Err("canvas font output exceeded admission".into()); }
    shorthand.to_css(&mut style_traits::CssWriter::new(&mut FontBoundedWriter { text: &mut value, limit: count.bytes })).map_err(|e| e.to_string())?;
    admission.checkpoint()?;
    Ok(NativeCanvasFontSerialization { value, _allocation: allocation })
}
/// Serialized native shaping longhands, retained with their output admission.
/// Consumers resolve relative values in their native canvas style context;
/// this projection does not approximate font sizes or choose fallback fonts.
pub struct NativeFontStyleProjection {
    pub declarations: Vec<(String, String)>,
    _allocation: LeaseArc<dyn Send + Sync>,
}
/// Project computed native font longhands for the shared Canvas shaper. The
/// specified shorthand must not be reused here: relative sizes are already
/// resolved against the Canvas font source and must remain absolute.
pub fn project_computed_canvas_font(
    computed: &style::properties::ComputedValues,
    admission: &dyn ImportAdmission,
) -> Result<NativeFontStyleProjection, String> {
    use style::properties::{ShorthandId, SourcePropertyDeclaration};
    admission.checkpoint()?;
    let count = ShorthandId::Font.longhands().count();
    let allocation = admission.reserve(count.checked_mul(std::mem::size_of::<style::properties::PropertyDeclaration>())
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<SourcePropertyDeclaration>()))
        .ok_or("computed canvas font projection overflow")?)?;
    let mut declarations = SourcePropertyDeclaration::default();
    for id in ShorthandId::Font.longhands() {
        admission.checkpoint()?;
        declarations.declarations.push(computed.computed_or_resolved_declaration(id, None));
    }
    let font = NativeFontShorthand { declarations, _allocation: allocation };
    font.style_projection(admission)
}
fn write_native_font_value<W: std::fmt::Write>(declaration: &style::properties::PropertyDeclaration, writer: &mut style_traits::CssWriter<W>) -> Result<bool, std::fmt::Error> {
    use style::properties::PropertyDeclaration as P;
    use style_traits::ToCss;
    // Match the native typed values used by the shaping consumer, rather than
    // allocating through PropertyDeclaration's String-only serializer.
    macro_rules! values { ($($variant:ident),+ $(,)?) => { match declaration {
        $(P::$variant(value) => { value.to_css(writer)?; Ok(true) },)+
        _ => Ok(false),
    } }; }
    values!(FontSize, FontFamily, FontStyle, FontWeight, FontStretch, FontVariantCaps,
        FontKerning, FontFeatureSettings, FontVariationSettings, FontOpticalSizing,
        FontSizeAdjust, FontLanguageOverride, FontVariantLigatures, FontVariantNumeric,
        FontVariantEastAsian, FontVariantPosition, LineHeight)
}
impl NativeFontShorthand {
    pub fn supports_canvas_syntax(&self) -> bool {
        !self.declarations.declarations.iter().any(|declaration| matches!(declaration,
            style::properties::PropertyDeclaration::CSSWideKeyword(..) | style::properties::PropertyDeclaration::WithVariables(..)))
    }
    pub fn style_projection(&self, admission: &dyn ImportAdmission) -> Result<NativeFontStyleProjection, String> {
        admission.checkpoint()?;
        let count = self.declarations.declarations.len();
        let mut bytes = count.checked_mul(std::mem::size_of::<(String, String)>()).ok_or("font projection overflow")?;
        for declaration in &self.declarations.declarations {
            let mut value = FontDescriptorCount::default();
            if !write_native_font_value(declaration, &mut style_traits::CssWriter::new(&mut value)).map_err(|e| e.to_string())? { continue; }
            bytes = bytes.checked_add(declaration.id().name().len()).and_then(|n| n.checked_add(value.bytes)).ok_or("font projection overflow")?;
            admission.checkpoint()?;
        }
        let allocation = admission.reserve(bytes)?;
        admission.checkpoint()?;
        let mut declarations = Vec::new();
        declarations.try_reserve_exact(count).map_err(|e| e.to_string())?;
        if declarations.capacity() > count { return Err("font projection exceeded admission".into()); }
        for declaration in &self.declarations.declarations {
            admission.checkpoint()?;
            let mut count = FontDescriptorCount::default();
            if !write_native_font_value(declaration, &mut style_traits::CssWriter::new(&mut count)).map_err(|e| e.to_string())? { continue; }
            let name = declaration.id().name();
            let mut key = String::new();
            key.try_reserve_exact(name.len()).map_err(|e| e.to_string())?;
            if key.capacity() > name.len() { return Err("font key exceeded admission".into()); }
            key.push_str(&name);
            let mut value = String::new();
            value.try_reserve_exact(count.bytes).map_err(|e| e.to_string())?;
            if value.capacity() > count.bytes { return Err("font value exceeded admission".into()); }
            write_native_font_value(declaration, &mut style_traits::CssWriter::new(&mut FontBoundedWriter { text: &mut value, limit: count.bytes })).map_err(|e| e.to_string())?;
            declarations.push((key, value));
        }
        admission.checkpoint()?;
        Ok(NativeFontStyleProjection { declarations, _allocation: allocation })
    }
    pub fn axes(&self) -> Result<NativeFontAxes, String> {
        use style::properties::PropertyDeclaration as P;
        use style::values::specified::font::{FontStretch, FontStyle, FontWeight};
        let mut axes = NativeFontAxes {
            weight: (400., 400.),
            width: (100., 100.),
            style: NativeFontStyle::Normal,
        };
        for d in &self.declarations.declarations {
            match d {
                P::FontWeight(FontWeight::Absolute(w)) => {
                    let w = w.compute().ok_or("unresolved font weight")?.value();
                    axes.weight = (w, w);
                }
                P::FontStretch(FontStretch::Keyword(k)) => {
                    let w = k.compute().to_percentage().0 * 100.;
                    axes.width = (w, w);
                }
                P::FontStretch(FontStretch::Stretch(p)) => {
                    let w = p.compute().ok_or("unresolved font stretch")?.0 * 100.;
                    axes.width = (w, w);
                }
                P::FontStyle(FontStyle::Specified(s)) => {
                    use style::values::generics::font::FontStyle as S;
                    axes.style = match s {
                        S::Italic => NativeFontStyle::Italic,
                        S::Oblique(a) => {
                            let a = a.degrees().ok_or("unresolved angle")?;
                            if a == 0. {
                                NativeFontStyle::Normal
                            } else {
                                NativeFontStyle::Oblique(a, a)
                            }
                        }
                    };
                }
                _ => {}
            }
        }
        Ok(axes)
    }
    pub fn family_matches(&self, family: &str) -> bool {
        use style::properties::PropertyDeclaration;
        use style::values::{computed::font::SingleFontFamily, specified::font::FontFamily};
        self.declarations.declarations.iter().any(|d|match d {
            PropertyDeclaration::FontFamily(value)=>match value {
                FontFamily::Values(list)=>list.iter().any(|f|matches!(f,SingleFontFamily::FamilyName(n) if <str>::eq_ignore_ascii_case(n.name.as_ref(), family))),
                _=>false,
            },
            _=>false,
        })
    }
    pub fn family_count(&self) -> usize {
        use style::properties::PropertyDeclaration;
        use style::values::specified::font::FontFamily;
        self.declarations.declarations.iter().find_map(|d| match d {
            PropertyDeclaration::FontFamily(FontFamily::Values(list)) => Some(list.iter().count()),
            _ => None,
        }).unwrap_or(0)
    }
    pub fn family_matches_at(&self, index: usize, family: &str) -> bool {
        use style::properties::PropertyDeclaration;
        use style::values::{computed::font::SingleFontFamily, specified::font::FontFamily};
        self.declarations.declarations.iter().any(|d| match d {
            PropertyDeclaration::FontFamily(FontFamily::Values(list)) => list.iter().nth(index)
                .is_some_and(|f| matches!(f, SingleFontFamily::FamilyName(n) if <str>::eq_ignore_ascii_case(n.name.as_ref(), family))),
            _ => false,
        })
    }
}
#[cfg(test)]
mod constructed_font_controls {
    use super::*;
    struct Admission;
    impl ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            Ok(LeaseArc::new(()))
        }
    }
    #[test]
    fn native_font_style_projection_preserves_parsed_longhands_and_admission() {
        let font = parse_native_font_shorthand_typed("italic bold 20px/2 Example, serif", "https://example.test/", &Admission).unwrap();
        let projection = font.style_projection(&Admission).unwrap();
        let get = |name: &str| projection.declarations.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str()).unwrap();
        assert_eq!(get("font-size"), "20px");
        assert_eq!(get("font-style"), "italic");
        assert_eq!(get("font-weight"), "bold");
        assert_eq!(get("line-height"), "2");
        assert!(get("font-family").contains("Example"));
        let style = projection.declarations.iter().cloned().collect();
        let shape = crate::canvas_text::shape("Ag", &style, Default::default(), &[]).unwrap();
        assert_eq!(shape.font_size, 20.);
        assert!(shape.width > 0.);
        struct Denied;
        impl ImportAdmission for Denied {
            fn checkpoint(&self) -> Result<(), String> { Ok(()) }
            fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> { Err("denied".into()) }
        }
        assert!(font.style_projection(&Denied).is_err());
    }
    #[test]
    fn native_font_style_projection_retains_charge_and_discards_every_cancelled_stage() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Charge(LeaseArc<AtomicUsize>);
        impl Drop for Charge { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
        struct Controlled { live: LeaseArc<AtomicUsize>, checks: AtomicUsize, deny_at: usize }
        impl ImportAdmission for Controlled {
            fn checkpoint(&self) -> Result<(), String> {
                if self.checks.fetch_add(1, Ordering::SeqCst) >= self.deny_at { Err("cancelled".into()) } else { Ok(()) }
            }
            fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
                self.live.fetch_add(1, Ordering::SeqCst);
                Ok(LeaseArc::new(Charge(self.live.clone())))
            }
        }
        let font = parse_native_font_shorthand_typed("italic bold 20px/2 Example, serif", "https://example.test/", &Admission).unwrap();
        let admission = Controlled { live: LeaseArc::new(AtomicUsize::new(0)), checks: AtomicUsize::new(0), deny_at: usize::MAX };
        let projection = font.style_projection(&admission).unwrap();
        assert_eq!(admission.live.load(Ordering::SeqCst), 1);
        let stages = admission.checks.load(Ordering::SeqCst);
        assert!(stages > 3);
        drop(projection);
        assert_eq!(admission.live.load(Ordering::SeqCst), 0);
        for deny_at in 0..stages {
            let admission = Controlled { deny_at, live: admission.live.clone(), checks: AtomicUsize::new(0) };
            assert!(font.style_projection(&admission).is_err(), "cancel checkpoint {deny_at}");
            assert_eq!(admission.live.load(Ordering::SeqCst), 0, "leaked checkpoint {deny_at}");
        }
    }
    #[test]
    fn constructed_descriptors_keep_identity_and_invalid_setter_is_atomic() {
        let mut face = NativeConstructedFontFace::parse(
            "Example",
            "url(face.woff2)",
            &[("weight", "400")],
            "https://example.test/",
            &Admission,
        );
        // Descriptor names use CSS native identifiers, not JS dictionary keys.
        assert!(face.as_ref().unwrap().initial_error.is_some());
        face = NativeConstructedFontFace::parse(
            "Example",
            "url(face.woff2)",
            &[("font-weight", "400")],
            "https://example.test/",
            &Admission,
        );
        let mut face = face.unwrap();
        let identity = face.identity();
        assert!(
            face.set_descriptor("font-weight", "invalid", &Admission)
                .is_err()
        );
        assert_eq!(face.revision(), 0);
        assert!(
            face.set_descriptor("font-weight", "700", &Admission)
                .unwrap()
        );
        assert_eq!(face.identity(), identity);
        assert_eq!(face.demand(&Admission).unwrap().family, "Example");
    }
    #[test]
    fn shorthand_uses_native_property_parser_and_exact_family_list() {
        let query = parse_native_font_shorthand(
            "italic 700 16px 'Example, Face', Other",
            "https://example.test/",
            &Admission,
        )
        .unwrap();
        assert!(query.family_matches("Example, Face"));
        assert!(query.family_matches("Other"));
        assert!(!query.family_matches("Example"));
        assert!(
            parse_native_font_shorthand("not a font", "https://example.test/", &Admission).is_err()
        );
    }
    #[test]
    fn nearest_axes_share_rendering_weight_and_width_precedence() {
        let query =
            parse_native_font_shorthand("400 16px Example", "https://example.test/", &Admission)
                .unwrap()
                .axes()
                .unwrap();
        let low = NativeConstructedFontFace::parse(
            "Example",
            "local(Example)",
            &[("font-weight", "300")],
            "https://example.test/",
            &Admission,
        )
        .unwrap();
        let high = NativeConstructedFontFace::parse(
            "Example",
            "local(Example)",
            &[("font-weight", "500")],
            "https://example.test/",
            &Admission,
        )
        .unwrap();
        assert!(
            native_font_axes_rank(high.axes().unwrap(), query)
                < native_font_axes_rank(low.axes().unwrap(), query)
        );
        assert_eq!(
            high.descriptor("font-weight", &Admission).unwrap().value,
            "500"
        );
        let condensed = NativeConstructedFontFace::parse(
            "Example",
            "local(Example)",
            &[("font-weight", "400"), ("font-stretch", "condensed")],
            "https://example.test/",
            &Admission,
        )
        .unwrap();
        assert!(
            native_font_axes_rank(high.axes().unwrap(), query)
                < native_font_axes_rank(condensed.axes().unwrap(), query)
        );
    }
    #[test]
    fn changed_descriptor_releases_superseded_lease() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Charge(LeaseArc<AtomicUsize>);
        impl Drop for Charge {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        struct Counting(LeaseArc<AtomicUsize>);
        impl ImportAdmission for Counting {
            fn checkpoint(&self) -> Result<(), String> {
                Ok(())
            }
            fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(LeaseArc::new(Charge(self.0.clone())))
            }
        }
        let live = LeaseArc::new(AtomicUsize::new(0));
        let admission = Counting(live.clone());
        let mut face = NativeConstructedFontFace::parse(
            "Example",
            "local(Example)",
            &[],
            "https://example.test/",
            &admission,
        )
        .unwrap();
        face.set_descriptor("font-weight", "400", &admission)
            .unwrap();
        let expected = live.load(Ordering::SeqCst);
        for value in ["500", "600", "700", "800"] {
            face.set_descriptor("font-weight", value, &admission)
                .unwrap();
            assert_eq!(live.load(Ordering::SeqCst), expected);
        }
        drop(face);
        assert_eq!(live.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn commit_receipt_releases_charge_only_after_outer_borrow() {
        use std::cell::RefCell;
        let admission = Admission;
        let face = RefCell::new(
            NativeConstructedFontFace::parse(
                "Example",
                "local(Example)",
                &[],
                "https://example.test/",
                &admission,
            )
            .unwrap(),
        );
        let prepared = face
            .borrow()
            .edit_context()
            .prepare("font-weight", "700", &admission)
            .unwrap();
        let mut borrow = face.borrow_mut();
        let receipt = match borrow.commit_descriptor(prepared) {
            Ok(receipt) => receipt,
            Err(_) => panic!("fresh prepared edit rejected"),
        };
        assert!(receipt.changed);
        assert!(receipt.obsolete_descriptor.is_some());
        drop(borrow);
        drop(receipt);
        let prepared = face
            .borrow()
            .edit_context()
            .prepare("font-weight", "700", &admission)
            .unwrap();
        let mut borrow = face.borrow_mut();
        let receipt = match borrow.commit_descriptor(prepared) {
            Ok(receipt) => receipt,
            Err(_) => panic!("fresh no-op rejected"),
        };
        assert!(!receipt.changed);
        assert!(receipt.obsolete_lease.is_some());
        drop(borrow);
        drop(receipt);
    }
}
#[derive(Debug)]
pub enum NativeFontShorthandError { Syntax, Resource(String) }
pub fn parse_native_font_shorthand(
    font: &str,
    base: &str,
    admission: &dyn ImportAdmission,
) -> Result<NativeFontShorthand, String> {
    parse_native_font_shorthand_typed(font,base,admission).map_err(|error|match error{
        NativeFontShorthandError::Syntax=>"invalid font shorthand".into(),
        NativeFontShorthandError::Resource(message)=>message,
    })
}
pub fn parse_native_font_shorthand_typed(
    font:&str,base:&str,admission:&dyn ImportAdmission,
)->Result<NativeFontShorthand,NativeFontShorthandError>{
    use style::properties::{PropertyId, SourcePropertyDeclaration};
    let allocation = admission.reserve(
        font.len()
            .checked_add(base.len())
            .and_then(|n| n.checked_add(std::mem::size_of::<SourcePropertyDeclaration>()))
            .ok_or_else(||NativeFontShorthandError::Resource("font shorthand overflow".into()))?,
    ).map_err(NativeFontShorthandError::Resource)?;
    admission.checkpoint().map_err(NativeFontShorthandError::Resource)?;
    let base = UrlExtraData::from(url::Url::parse(base).map_err(|e| NativeFontShorthandError::Resource(e.to_string()))?);
    let mut declarations = SourcePropertyDeclaration::default();
    style::properties::declaration_block::parse_one_declaration_into(
        &mut declarations,
        PropertyId::parse_enabled_for_all_content("font").map_err(|_| NativeFontShorthandError::Resource("font shorthand absent".into()))?,
        font,
        Origin::Author,
        &base,
        None,
        style_traits::ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        style::stylesheets::CssRuleType::Style,
    )
    .map_err(|_|NativeFontShorthandError::Syntax)?;
    admission.checkpoint().map_err(NativeFontShorthandError::Resource)?;
    Ok(NativeFontShorthand {
        declarations,
        _allocation: allocation,
    })
}

pub struct NativeConstructedEditContext {
    identity: LeaseArc<Mutex<style::font_face::Descriptors>>,
    base: UrlExtraData,
    revision: u64,
    epoch: LeaseArc<std::sync::atomic::AtomicU64>,
}
pub struct PreparedConstructedDescriptor {
    identity: LeaseArc<Mutex<style::font_face::Descriptors>>,
    id: style::font_face::DescriptorId,
    slot: usize,
    expected_revision: u64,
    next_revision: u64,
    staged: style::font_face::Descriptors,
    allocation: LeaseArc<dyn Send + Sync>,
}
impl NativeConstructedEditContext {
    pub fn prepare(
        self,
        name: &str,
        value: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<PreparedConstructedDescriptor, String> {
        use style::font_face::{DescriptorId, Descriptors};
        let id = DescriptorId::from_ident(name).map_err(|_| "unknown FontFace descriptor")?;
        let slot = (0..DescriptorId::COUNT)
            .find(|&index| DescriptorId::at(index) == Some(id))
            .ok_or("native descriptor index absent")?;
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or("constructed font revision exhausted")?;
        let allocation = admission.reserve(
            value
                .len()
                .checked_add(std::mem::size_of::<Descriptors>())
                .ok_or("constructed descriptor overflow")?,
        )?;
        let context = font_constructed_context(&self.base);
        let mut staged = Descriptors::default();
        let mut input = cssparser::ParserInput::new(value);
        staged
            .set(id, &context, &mut cssparser::Parser::new(&mut input))
            .map_err(|_| "invalid FontFace descriptor".to_owned())?;
        admission.checkpoint()?;
        Ok(PreparedConstructedDescriptor {
            identity: self.identity,
            id,
            slot,
            expected_revision: self.revision,
            next_revision,
            staged,
            allocation,
        })
    }
}
impl NativeConstructedDescriptor {
    pub fn into_parts(self) -> (String, LeaseArc<dyn Send + Sync>) {
        (self.value, self._allocation)
    }
}
impl NativeConstructedFontDemand {
    pub fn allocation(&self) -> LeaseArc<dyn Send + Sync> {
        self._allocation.clone()
    }
}

impl NativeConstructedEditContext {
    pub fn demand(
        &self,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeConstructedFontDemand, String> {
        use style_traits::ToCss;
        let revision = self.revision;
        let (text_bytes, bytes, count, ranges) = {
            let d = self
                .identity
                .lock()
                .map_err(|_| "constructed font lock poisoned")?;
            let mut text = FontDescriptorCount::default();
            d.to_css(&mut style_traits::CssWriter::new(&mut text))
                .map_err(|e| e.to_string())?;
            let mut bytes = d
                .font_family
                .as_ref()
                .map_or(0, |f| f.name.len())
                .checked_add(self.base.as_str().len())
                .ok_or("font demand overflow")?;
            for source in d.src.as_ref().map_or(&[][..], |s| &s.0[..]) {
                let source_bytes = match source {
                    style::font_face::Source::Local(n) => n.name.len(),
                    style::font_face::Source::Url(u) => u
                        .url
                        .url()
                        .map_or(0, |u| u.as_str().len())
                        .checked_add(match &u.format_hint {
                            Some(NativeFontSourceFormat::String(s)) => s.len(),
                            _ => 0,
                        })
                        .ok_or("font source overflow")?,
                };
                bytes = bytes
                    .checked_add(source_bytes)
                    .ok_or("font source overflow")?;
            }
            (
                text.bytes,
                bytes,
                d.src.as_ref().map_or(0, |s| s.0.len()),
                d.unicode_range.as_ref().map_or(1, Vec::len),
            )
        };
        let bytes = bytes
            .checked_add(text_bytes)
            .and_then(|n| {
                n.checked_add(count.checked_mul(std::mem::size_of::<NativeFontSource>())?)
            })
            .and_then(|n| n.checked_add(ranges.checked_mul(std::mem::size_of::<(u32, u32)>())?))
            .ok_or("font demand overflow")?;
        let allocation = admission.reserve(bytes)?;
        admission.checkpoint()?;
        let copy = |s: &str| -> Result<String, String> {
            let mut out = String::new();
            out.try_reserve_exact(s.len()).map_err(|e| e.to_string())?;
            out.push_str(s);
            Ok(out)
        };
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(count)
            .map_err(|e| e.to_string())?;
        let mut unicode_ranges = Vec::new();
        unicode_ranges
            .try_reserve_exact(ranges)
            .map_err(|e| e.to_string())?;
        let mut descriptors = String::new();
        descriptors
            .try_reserve_exact(text_bytes)
            .map_err(|e| e.to_string())?;
        let d = self
            .identity
            .lock()
            .map_err(|_| "constructed font lock poisoned")?;
        if self.epoch.load(std::sync::atomic::Ordering::Acquire) != revision {
            return Err("constructed font demand changed".into());
        }
        d.to_css(&mut style_traits::CssWriter::new(&mut FontBoundedWriter {
            text: &mut descriptors,
            limit: text_bytes,
        }))
        .map_err(|e| e.to_string())?;
        let family = copy(d.font_family.as_ref().map_or("", |f| f.name.as_ref()))?;
        for source in d.src.as_ref().map_or(&[][..], |s| &s.0[..]) {
            sources.push(match source {
                style::font_face::Source::Local(n) => NativeFontSource::Local {
                    name: copy(n.name.as_ref())?,
                },
                style::font_face::Source::Url(u) => {
                    let Some(url) = u.url.url() else { continue };
                    if !native_font_source_supported(u.format_hint.as_ref(), u.tech_flags.bits()) {
                        continue;
                    }
                    let format = match &u.format_hint {
                        None => None,
                        Some(NativeFontSourceFormat::Keyword(k)) => {
                            Some(NativeFontSourceFormat::Keyword(*k))
                        }
                        Some(NativeFontSourceFormat::String(s)) => {
                            Some(NativeFontSourceFormat::String(copy(s)?))
                        }
                    };
                    NativeFontSource::Url {
                        url: copy(url.as_str())?,
                        format,
                        tech: u.tech_flags.bits(),
                    }
                }
            });
        }
        if let Some(r) = &d.unicode_range {
            for r in r {
                unicode_ranges.push((r.start, r.end));
            }
        } else {
            unicode_ranges.push((0, 0x10ffff));
        }
        if self.epoch.load(std::sync::atomic::Ordering::Acquire) != revision {
            return Err("constructed font demand changed".into());
        }
        Ok(NativeConstructedFontDemand {
            identity: LeaseArc::as_ptr(&self.identity) as usize,
            revision,
            family,
            descriptors,
            sources,
            unicode_ranges,
            base_url: copy(self.base.as_str())?,
            _allocation: allocation,
        })
    }
    pub fn descriptor(
        &self,
        name: &str,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeConstructedDescriptor, String> {
        let id = style::font_face::DescriptorId::from_ident(name)
            .map_err(|_| "unknown FontFace descriptor")?;
        let bytes = {
            let d = self.identity.lock().map_err(|_| "font lock poisoned")?;
            let mut count = FontDescriptorCount::default();
            d.get_with_writer(id, &mut count)
                .map_err(|e| e.to_string())?;
            count.bytes
        };
        let allocation = admission.reserve(bytes)?;
        admission.checkpoint()?;
        let mut value = String::new();
        value.try_reserve_exact(bytes).map_err(|e| e.to_string())?;
        let d = self.identity.lock().map_err(|_| "font lock poisoned")?;
        d.get_with_writer(
            id,
            &mut FontBoundedWriter {
                text: &mut value,
                limit: bytes,
            },
        )
        .map_err(|e| e.to_string())?;
        if self.epoch.load(std::sync::atomic::Ordering::Acquire) != self.revision {
            return Err("constructed descriptor read changed".into());
        }
        Ok(NativeConstructedDescriptor {
            value,
            _allocation: allocation,
        })
    }
    pub fn axes(&self) -> Result<NativeFontAxes, String> {
        let d = self.identity.lock().map_err(|_| "font lock poisoned")?;
        if self.epoch.load(std::sync::atomic::Ordering::Acquire) != self.revision {
            return Err("constructed font axes changed".into());
        }
        font_descriptor_axes(&d)
    }
}

/// Caller drops its RefMut before this receipt. Explicit Drop destroys old
/// parsed descriptors before releasing the corresponding admission lease.
pub struct NativeConstructedCommit {
    pub changed: bool,
    obsolete_descriptor: Option<style::font_face::Descriptors>,
    obsolete_lease: Option<LeaseArc<dyn Send + Sync>>,
}
impl Drop for NativeConstructedCommit {
    fn drop(&mut self) {
        drop(self.obsolete_descriptor.take());
        drop(self.obsolete_lease.take());
    }
}

impl NativeConstructedEditContext {
    pub fn family(
        &self,
        admission: &dyn ImportAdmission,
    ) -> Result<NativeConstructedDescriptor, String> {
        let bytes = {
            let d = self.identity.lock().map_err(|_| "font lock poisoned")?;
            d.font_family.as_ref().map_or(0, |f| f.name.len())
        };
        let allocation = admission.reserve(bytes)?;
        admission.checkpoint()?;
        let mut value = String::new();
        value.try_reserve_exact(bytes).map_err(|e| e.to_string())?;
        let d = self.identity.lock().map_err(|_| "font lock poisoned")?;
        if self.epoch.load(std::sync::atomic::Ordering::Acquire) != self.revision {
            return Err("constructed family read stale".into());
        }
        if let Some(f) = &d.font_family {
            if f.name.len() > bytes {
                return Err("constructed family size stale".into());
            }
            value.push_str(f.name.as_ref());
        }
        Ok(NativeConstructedDescriptor {
            value,
            _allocation: allocation,
        })
    }
    pub fn intersects_text(&self, text: &str) -> Result<bool, String> {
        let d = self.identity.lock().map_err(|_| "font lock poisoned")?;
        if self.epoch.load(std::sync::atomic::Ordering::Acquire) != self.revision {
            return Err("constructed range read stale".into());
        }
        Ok(text.chars().any(|ch| {
            d.unicode_range.as_ref().is_none_or(|ranges| {
                ranges
                    .iter()
                    .any(|r| r.start <= ch as u32 && ch as u32 <= r.end)
            })
        }))
    }
}

#[cfg(test)]
mod binary_font_constructor_controls {
    use super::*;
    struct Admission;
    impl ImportAdmission for Admission {
        fn checkpoint(&self) -> Result<(), String> {
            Ok(())
        }
        fn reserve(&self, _: usize) -> Result<LeaseArc<dyn Send + Sync>, String> {
            Ok(LeaseArc::new(()))
        }
    }
    #[test]
    fn binary_constructor_has_no_synthetic_css_source() {
        let face = NativeConstructedFontFace::parse_binary(
            "Binary Family",
            &[("font-weight", "700")],
            "https://example.test/",
            &Admission,
        )
        .unwrap();
        assert!(face.initial_error.is_none());
        assert_eq!(
            face.edit_context().family(&Admission).unwrap().value,
            "Binary Family"
        );
        assert_eq!(face.descriptor("src", &Admission).unwrap().value, "");
        assert_eq!(
            face.descriptor("font-weight", &Admission).unwrap().value,
            "700"
        );
        assert!(face.demand(&Admission).unwrap().sources.is_empty());
    }
    #[test]
    fn binary_invalid_descriptor_records_initial_error_without_constructor_throw() {
        let face = NativeConstructedFontFace::parse_binary(
            "Binary",
            &[("font-weight", "invalid")],
            "https://example.test/",
            &Admission,
        )
        .unwrap();
        assert!(face.initial_error.is_some());
        assert_eq!(face.descriptor("src", &Admission).unwrap().value, "");
    }
}
