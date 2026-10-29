//! Native canvas text anchor state, shared by onscreen and offscreen contexts.
use super::*;
struct CanvasLanguage {
    raw: v8::Global<v8::String>,
    owned: font_loading::FontOwnedInput,
    _allocation: Option<std::sync::Arc<dyn Send + Sync>>,
}
impl std::fmt::Debug for CanvasLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("CanvasLanguage") }
}

/// Raw WebIDL string conversion, before CSS or Canvas whitespace processing.
/// The shared UTF-16 converter preserves CR/LF/NUL; only its CSS consumers
/// subsequently apply CSS Syntax preprocessing. Text consumers must not pass
/// this value through the font parser. Operations perform ToString and receiver
/// revalidation before calling this helper with the immutable resulting string.
/// Check the document/stream again around native allocation preparation.
pub(super) fn owned_string(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::String>,
) -> Result<font_loading::FontOwnedInput, V8BindingError> {
    let identity = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
    );
    let text = font_loading::font_owned_input(scope, state, value.into())?;
    if state.execution_tasks_stopped()
        || identity
            != (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
            )
    {
        return Err(V8BindingError::Message("InvalidStateError".into()));
    }
    Ok(text)
}

#[derive(Debug, Clone)]
pub(super) struct TextState {
    pub(super) font: Option<Rc<oryn_layout::CanvasFontResolution>>,
    direction: &'static str,
    align: &'static str,
    baseline: &'static str,
    kerning: &'static str,
    stretch: &'static str,
    caps: &'static str,
    rendering: &'static str,
    letter_spacing: Option<Rc<oryn_layout::cssom::native_import_graph::NativeCanvasSpacing>>,
    word_spacing: Option<Rc<oryn_layout::cssom::native_import_graph::NativeCanvasSpacing>>,
    language: Option<Rc<CanvasLanguage>>,
}
impl Default for TextState {
    fn default() -> Self {
        Self {
            font: None,
            direction: "inherit",
            align: "start",
            baseline: "alphabetic",
            kerning: "auto", stretch: "normal", caps: "normal", rendering: "auto",
            letter_spacing: None, word_spacing: None,
            language: None,
        }
    }
}
impl TextState {
    pub(super) fn capture_language(&self, scope: &mut v8::PinScope, state: &Rc<V8BindingState>, canvas: NodeId, source_element: bool)
        -> Result<(Option<String>, Option<std::sync::Arc<dyn Send + Sync>>), V8BindingError> {
        let explicit = self.language.as_ref().filter(|language| language.owned.value != "inherit");
        let source = if source_element { canvas } else { state.dom.borrow().document_element().unwrap_or(state.document_root()) };
        let size = if let Some(language) = explicit { Some(language.owned.value.len()) }
            else { state.dom.borrow().element_language(source)?.map(str::len) };
        let Some(size) = size else { return Ok((None, None)); };
        let allocation = font_loading::reserve_font_container(scope, state, size)?;
        let mut value = String::new();
        value.try_reserve_exact(size).map_err(|_| V8BindingError::Allocation("Canvas language projection"))?;
        if value.capacity() > size { return Err(V8BindingError::Allocation("Canvas language projection admission")); }
        if let Some(language) = explicit { value.push_str(&language.owned.value); }
        else {
            let dom = state.dom.borrow();
            let language = dom.element_language(source)?.unwrap_or("");
            if language.len() > size { return Err(V8BindingError::Allocation("Canvas inherited language changed during admission")); }
            value.push_str(language);
        }
        Ok((Some(value), allocation))
    }
    fn value(&self, name: &str) -> &'static str {
        match name {
            "direction" => self.direction,
            "textAlign" => self.align,
            "fontKerning" => self.kerning,
            "fontStretch" => self.stretch,
            "fontVariantCaps" => self.caps,
            "textRendering" => self.rendering,
            _ => self.baseline,
        }
    }
    fn choices(name: &str) -> &'static [&'static str] {
        match name {
            "direction" => &["ltr", "rtl", "inherit"],
            "textAlign" => &["left", "right", "center", "start", "end"],
            "textBaseline" => &["top", "hanging", "middle", "alphabetic", "ideographic", "bottom"],
            "fontKerning" => &["auto", "normal", "none"],
            "fontStretch" => &["ultra-condensed", "extra-condensed", "condensed", "semi-condensed", "normal", "semi-expanded", "expanded", "extra-expanded", "ultra-expanded"],
            "fontVariantCaps" => &["normal", "small-caps", "all-small-caps", "petite-caps", "all-petite-caps", "unicase", "titling-caps"],
            "textRendering" => &["auto", "optimizeSpeed", "optimizeLegibility", "geometricPrecision"],
            _ => &[],
        }
    }
    fn set(&mut self, name: &str, value: &str) {
        let Some(value) = Self::choices(name).iter().copied().find(|choice| *choice == value) else { return; };
        match name {
            "direction" => self.direction = value,
            "textAlign" => self.align = value,
            "textBaseline" => self.baseline = value,
            "fontKerning" => self.kerning = value,
            "fontStretch" => self.stretch = value,
            "fontVariantCaps" => self.caps = value,
            "textRendering" => self.rendering = value,
            _ => {}
        }
    }
    pub(super) fn apply_shaping(&self, style: &mut BTreeMap<String, String>) {
        style.insert("letter-spacing".into(), self.letter_spacing.as_ref().map_or("0px", |value| value.css.as_str()).into());
        style.insert("word-spacing".into(), self.word_spacing.as_ref().map_or("0px", |value| value.css.as_str()).into());
        style.insert("font-kerning".into(), self.kerning.into());
        style.insert("text-rendering".into(), self.rendering.into());
        if self.stretch != "normal" { style.insert("font-stretch".into(), self.stretch.into()); }
        // A normal caps override leaves the font shorthand's own caps in force.
        if self.caps != "normal" { style.insert("font-variant-caps".into(), self.caps.into()); }
    }
    pub(super) fn shaping_projection_bytes(&self) -> Option<usize> {
        [("font-kerning", self.kerning), ("text-rendering", self.rendering),
            ("font-stretch", self.stretch), ("font-variant-caps", self.caps),
            ("letter-spacing", self.letter_spacing.as_ref().map_or("0px", |value| value.css.as_str())),
            ("word-spacing", self.word_spacing.as_ref().map_or("0px", |value| value.css.as_str()))]
            .into_iter().try_fold(0usize, |bytes, (key, value)| bytes.checked_add(128)
                .and_then(|bytes| bytes.checked_add(key.len())).and_then(|bytes| bytes.checked_add(value.len())))
    }
    pub(super) fn rtl(&self, inherited_rtl: bool) -> bool {
        match self.direction {
            "rtl" => true,
            "ltr" => false,
            _ => inherited_rtl,
        }
    }
    pub(super) fn alignment(&self) -> canvas_text_path::Alignment {
        use canvas_text_path::Alignment;
        match self.align {
            "left" => Alignment::Left,
            "right" => Alignment::Right,
            "center" => Alignment::Center,
            "end" => Alignment::End,
            _ => Alignment::Start,
        }
    }
    pub(super) fn baseline(&self) -> oryn_layout::canvas_text::metrics::Baseline {
        use oryn_layout::canvas_text::metrics::Baseline;
        match self.baseline {
            "top" => Baseline::Top,
            "hanging" => Baseline::Hanging,
            "middle" => Baseline::Middle,
            "ideographic" => Baseline::Ideographic,
            "bottom" => Baseline::Bottom,
            _ => Baseline::Alphabetic,
        }
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    for name in ["direction", "textAlign", "textBaseline", "font", "fontKerning", "fontStretch", "fontVariantCaps", "textRendering", "letterSpacing", "wordSpacing", "lang"] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("canvas text state key"))?;
        let getter = v8::Function::builder(get)
            .data(key.into())
            .length(0)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("canvas text state getter"))?;
        let setter = v8::Function::builder(set)
            .data(key.into())
            .length(1)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("canvas text state setter"))?;
        let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        if context.define_property(scope, key.into(), &descriptor) != Some(true) {
            return Err(V8BindingError::Allocation("canvas text state accessor"));
        }
    }
    Ok(())
}
fn receiver_state(scope: &mut v8::PinScope, args: &v8::FunctionCallbackArguments) -> Option<Rc<V8BindingState>> {
    let state = canvas_context_owner(scope, args.this()).and_then(|owner| binding_state_for_receiver(scope, owner));
    if state.is_none() { throw_type_error(scope, "Canvas text state requires a native 2D context"); }
    state
}
fn get(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(state) = receiver_state(scope, &args) else { return; };
    let Ok(canvas) = canvas_receiver_node(scope, &state, &args) else {
        return;
    };
    let name = args.data().to_rust_string_lossy(scope);
    if name == "lang" {
        let language = state.canvas_draw_states.borrow().get(&canvas).and_then(|draw| draw.text.language.clone());
        if let Some(language) = language { rv.set(v8::Local::new(scope, &language.raw).into()); }
        else if let Some(value) = v8::String::new(scope, "inherit") { rv.set(value.into()); }
        return;
    }
    if matches!(name.as_str(), "letterSpacing" | "wordSpacing") {
        let spacing = state.canvas_draw_states.borrow().get(&canvas).and_then(|draw| {
            if name == "letterSpacing" { draw.text.letter_spacing.clone() } else { draw.text.word_spacing.clone() }
        });
        if let Some(value) = v8::String::new(scope, spacing.as_ref().map_or("0px", |value| value.css.as_str())) { rv.set(value.into()); }
        return;
    }
    if name == "font" {
        let font = state
            .canvas_draw_states
            .borrow()
            .get(&canvas)
            .and_then(|draw| draw.text.font.clone());
        if let Some(value) = v8::String::new(
            scope,
            font.as_ref()
                .map_or("10px sans-serif", |font| font.css.value.as_str()),
        ) {
            rv.set(value.into());
        }
        return;
    }
    let value = state
        .canvas_draw_states
        .borrow()
        .get(&canvas)
        .map(|draw| draw.text.value(&name))
        .unwrap_or_else(|| TextState::default().value(&name));
    if let Some(value) = v8::String::new(scope, value) {
        rv.set(value.into());
    }
}
fn set(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let Some(state) = receiver_state(scope, &args) else { return; };
    let Ok(canvas) = canvas_receiver_node(scope, &state, &args) else {
        return;
    };
    let name = args.data().to_rust_string_lossy(scope);
    // ToString can run author code, resize/reset the surface or throw. Never
    // retain a draw-state borrow over that conversion; revalidate afterwards.
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    let Ok(current) = canvas_receiver_node(scope, &state, &args) else {
        return;
    };
    if current != canvas {
        return;
    }
    if name == "lang" {
        let result = (|| -> Result<(), V8BindingError> {
            let _entry = state.enter_retained_allocation(scope)?;
            let identity = (state.document_root(), state.document_generation(), state.document_stream_epoch());
            let owned = owned_string(scope, &state, value)?;
            let allocation = font_loading::reserve_font_container(scope, &state, std::mem::size_of::<CanvasLanguage>() + 2 * std::mem::size_of::<usize>())?;
            if state.execution_tasks_stopped() || identity != (state.document_root(), state.document_generation(), state.document_stream_epoch()) || canvas_receiver_node(scope, &state, &args)? != canvas { return Err(V8BindingError::Message("InvalidStateError".into())); }
            let language = Rc::new(CanvasLanguage { raw: v8::Global::new(scope, value), owned, _allocation: allocation });
            let old = state.canvas_draw_states.borrow_mut().entry(canvas).or_default().text.language.replace(language);
            drop(old);
            Ok(())
        })();
        if let Err(error) = result { throw_binding_error(scope, &error); }
        return;
    }
    if matches!(name.as_str(), "letterSpacing" | "wordSpacing") {
        let result = (|| -> Result<(), V8BindingError> {
            let _entry = state.enter_retained_allocation(scope)?;
            let identity = (state.document_root(), state.document_generation(), state.document_stream_epoch());
            let owned = owned_string(scope, &state, value)?;
            let base = state.native_retained_document_base_url(state.document_root())?;
            let allocation = state.retained_allocation.borrow().clone().ok_or(V8BindingError::Allocation("Canvas spacing admission unavailable"))?;
            let admission = SpacingAdmission { cancelled: allocation.capture_cancellation().map_err(V8BindingError::Message)?, allocation };
            let spacing = match oryn_layout::cssom::native_import_graph::parse_native_canvas_spacing(&owned.value, &base, &admission) {
                Ok(spacing) => spacing,
                Err(oryn_layout::cssom::native_import_graph::NativeFontShorthandError::Syntax) => return Ok(()),
                Err(oryn_layout::cssom::native_import_graph::NativeFontShorthandError::Resource(error)) => return Err(V8BindingError::Message(error)),
            };
            if scope.is_execution_terminating() { return Err(V8BindingError::JavaScriptException); }
            if state.execution_tasks_stopped() || identity != (state.document_root(), state.document_generation(), state.document_stream_epoch())
                || canvas_receiver_node(scope, &state, &args)? != canvas { return Err(V8BindingError::Message("InvalidStateError".into())); }
            let obsolete = {
                let mut states = state.canvas_draw_states.borrow_mut();
                let text = &mut states.entry(canvas).or_default().text;
                if name == "letterSpacing" { text.letter_spacing.replace(Rc::new(spacing)) }
                else { text.word_spacing.replace(Rc::new(spacing)) }
            };
            drop(obsolete);
            Ok(())
        })();
        if let Err(error) = result { throw_binding_error(scope, &error); }
        return;
    }
    if name == "font" {
        let result = (|| -> Result<(), V8BindingError> {
            let _entry = state.enter_retained_allocation(scope)?;
            let owned = owned_string(scope, &state, value)?;
            refresh_layout_in_scope(scope, &state)?;
            let provider = state
                .native_layout_provider()
                .ok_or(V8BindingError::Allocation("native canvas font provider"))?;
            let source_element = canvas_context_owner(scope, args.this())
                .is_some_and(|owner| offscreen_canvas::identify(scope, owner.into()).is_none());
            let request = BindingCanvasFontRequest {
                document: state.document_root(),
                canvas,
                document_generation: state.document_generation(),
                stream_epoch: state.document_stream_epoch(),
                style_generation: provider.layout_revision(),
                source_element,
            };
            let inputs = state.native_font_layout_inputs()?;
            // Logical admission for the transport-to-map projection, including
            // conservative B-tree node metadata and copied CSS state strings.
            let mut bytes = inputs
                .states
                .len()
                .checked_add(state.scroll_offsets.borrow().len())
                .and_then(|n| n.checked_mul(128))
                .ok_or(V8BindingError::Allocation("canvas font snapshot overflow"))?;
            for (_, values) in &inputs.states {
                for value in values {
                    bytes = bytes
                        .checked_add(128)
                        .and_then(|n| n.checked_add(value.len()))
                        .ok_or(V8BindingError::Allocation(
                            "canvas font state snapshot overflow",
                        ))?;
                }
            }
            let _maps = font_loading::reserve_font_container(scope, &state, bytes)?;
            let states = inputs
                .states
                .iter()
                .map(|(node, values)| (*node, values.iter().cloned().collect()))
                .collect();
            let result = provider
                .resolve_canvas_font(
                    &state.dom,
                    request,
                    &owned.value,
                    &states,
                    state.document_scroll_offset(),
                    &state.element_scroll_offsets(),
                )
                .ok_or(V8BindingError::Allocation(
                    "native canvas font provider unavailable",
                ))?;
            let font = match result {
                Ok(font) => font,
                Err(BindingCanvasFontError::Syntax) => return Ok(()),
                Err(BindingCanvasFontError::Native(error)) => {
                    return Err(V8BindingError::Message(error));
                }
            };
            if state.execution_retired()
                || state.execution_suspended()
                || state.document_root() != request.document
                || state.document_generation() != request.document_generation
                || state.document_stream_epoch() != request.stream_epoch
                || canvas_receiver_node(scope, &state, &args)? != canvas
            {
                return Err(V8BindingError::Message("InvalidStateError".into()));
            }
            let obsolete = {
                let mut states = state.canvas_draw_states.borrow_mut();
                let text = &mut states.entry(canvas).or_default().text;
                text.caps = "normal";
                text.font.replace(Rc::new(font))
            };
            drop(obsolete);
            Ok(())
        })();
        if let Err(error) = result {
            throw_binding_error(scope, &error);
        }
        return;
    }
    // All accepted tokens are ASCII. Reject an overlength converted value
    // before exporting it into an unbounded native String; ToString still ran.
    let max = TextState::choices(&name).iter().map(|value| value.len()).max().unwrap_or(0);
    if value.length() > max { return; }
    let value = value.to_rust_string_lossy(scope);
    state
        .canvas_draw_states
        .borrow_mut()
        .entry(canvas)
        .or_default()
        .text
        .set(&name, &value);
}

struct SpacingAdmission {
    allocation: std::sync::Arc<BindingRetainedAllocation>,
    cancelled: std::sync::Arc<dyn Fn() -> bool + Send + Sync>,
}
impl oryn_layout::cssom::native_import_graph::ImportAdmission for SpacingAdmission {
    fn checkpoint(&self) -> Result<(), String> {
        if (self.cancelled)() { Err("Canvas spacing owner retired".into()) } else { Ok(()) }
    }
    fn reserve(&self, bytes: usize) -> Result<std::sync::Arc<dyn Send + Sync>, String> {
        self.checkpoint()?;
        self.allocation.reserve(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_canvas_feature_projection_preserves_shorthand_caps_and_width_defaults() {
        let mut state = TextState::default();
        let mut style = BTreeMap::from([("font-variant-caps".into(), "small-caps".into()), ("font-stretch".into(), "condensed".into())]);
        state.apply_shaping(&mut style);
        assert_eq!(style["font-variant-caps"], "small-caps");
        assert_eq!(style["font-stretch"], "condensed");
        state.set("fontVariantCaps", "all-small-caps");state.set("fontStretch", "expanded");
        state.set("fontKerning", "none");state.set("textRendering", "optimizeSpeed");
        state.apply_shaping(&mut style);
        assert_eq!(style["font-variant-caps"], "all-small-caps");
        assert_eq!(style["font-stretch"], "expanded");
        assert_eq!(style["font-kerning"], "none");
        assert_eq!(style["text-rendering"], "optimizeSpeed");
    }
    #[test]
    fn native_canvas_owned_text_preserves_raw_whitespace_and_conversion_reentry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<canvas></canvas>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(scope, r#"globalThis.conversions=0;({toString(){conversions++;return 'a\r\n\t\f\u0000\u000b\uD800\uD83D\uDD25'}})"#).unwrap();
        let object = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let converted = object.to_string(scope).unwrap();
        let text = owned_string(scope, &state, converted).unwrap();
        assert_eq!(text.value, "a\r\n\t\u{c}\0\u{b}\u{fffd}\u{1f525}");
        let key = v8::String::new(scope, "conversions").unwrap();
        assert_eq!(
            context
                .global(scope)
                .get(scope, key.into())
                .unwrap()
                .integer_value(scope),
            Some(1)
        );
        drop(text);
        let source =
            v8::String::new(scope, "({toString(){throw new Error('text sentinel')}})").unwrap();
        let object = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        {
            v8::tc_scope!(let caught, scope);
            assert!(object.to_string(caught).is_none());
            assert!(caught.has_caught());
        }
        fn replace_stream(
            scope: &mut v8::PinScope,
            _: v8::FunctionCallbackArguments,
            mut rv: v8::ReturnValue,
        ) {
            let state = scope
                .get_current_context()
                .get_slot::<V8BindingState>()
                .unwrap();
            state.advance_document_stream_epoch().unwrap();
            rv.set(v8::String::new(scope, "replacement text").unwrap().into());
        }
        let object = v8::Object::new(scope);
        let key = v8::String::new(scope, "toString").unwrap();
        let callback = v8::Function::new(scope, replace_stream).unwrap();
        object.set(scope, key.into(), callback.into());
        let epoch = state.document_stream_epoch();
        let converted = object.to_string(scope).unwrap();
        assert_ne!(state.document_stream_epoch(), epoch);
        assert_eq!(
            owned_string(scope, &state, converted).unwrap().value,
            "replacement text"
        );
    }
    #[test]
    fn native_canvas_text_state_obeys_conversion_brand_and_save_restore() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<canvas id=c width=2 height=2></canvas>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(scope, r#"
            const c=document.getElementById('c'), onscreen=c.getContext('2d');
            for (const ctx of [onscreen,new OffscreenCanvas(2,2).getContext('2d')]) {
                for(const [name,initial,values] of [
                    ['direction','inherit',['ltr','rtl','inherit']],
                    ['textAlign','start',['left','right','center','start','end']],
                    ['textBaseline','alphabetic',['top','hanging','middle','alphabetic','ideographic','bottom']]
                ]) {
                    if(ctx[name]!==initial)throw Error('default '+name);
                    for(const value of values){ctx[name]=value;if(ctx[name]!==value)throw Error('value '+name);}
                    const kept=ctx[name];
                    for(const value of ['INVALID',' '+kept,kept.toUpperCase(),null,undefined]){
                        ctx[name]=value;if(ctx[name]!==kept)throw Error('invalid '+name);
                    }
                    const sentinel={};let caught;
                    try{ctx[name]={toString(){throw sentinel}}}catch(e){caught=e}
                    if(caught!==sentinel||ctx[name]!==kept)throw Error('conversion '+name);
                    let owner=ctx, descriptor;
                    while(owner&&!descriptor){descriptor=Object.getOwnPropertyDescriptor(owner,name);owner=Object.getPrototypeOf(owner);}
                    if(!descriptor.enumerable||!descriptor.configurable||descriptor.get.length!==0||descriptor.set.length!==1)throw Error('descriptor '+name);
                    let converted=false;
                    try{descriptor.set.call({}, {toString(){converted=true;return kept}});throw Error('brand accepted')}catch(e){if(!(e instanceof TypeError))throw e;}
                    if(converted)throw Error('brand conversion precedence');
                }
                ctx.textAlign='center';ctx.textBaseline='top';ctx.direction='rtl';ctx.save();
                ctx.textAlign='right';ctx.textBaseline='bottom';ctx.direction='ltr';ctx.restore();
                if(ctx.textAlign!=='center'||ctx.textBaseline!=='top'||ctx.direction!=='rtl')throw Error('restore');
                ctx.reset();
                if(ctx.textAlign!=='start'||ctx.textBaseline!=='alphabetic'||ctx.direction!=='inherit')throw Error('reset');
            }
            onscreen.direction='rtl';onscreen.save();
            onscreen.textAlign={toString(){c.width=2;return 'center'}};
            onscreen.restore();
            if(onscreen.direction!=='inherit'||onscreen.textAlign!=='center')throw Error('resize during conversion');
        "#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
}
