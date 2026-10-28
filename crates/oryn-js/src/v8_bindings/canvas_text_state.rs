//! Native canvas text anchor state, shared by onscreen and offscreen contexts.
use super::*;

#[derive(Debug, Clone)]
pub(super) struct TextState {
    direction: &'static str,
    align: &'static str,
    baseline: &'static str,
}
impl Default for TextState {
    fn default() -> Self {
        Self {
            direction: "inherit",
            align: "start",
            baseline: "alphabetic",
        }
    }
}
impl TextState {
    fn value(&self, name: &str) -> &'static str {
        match name {
            "direction" => self.direction,
            "textAlign" => self.align,
            _ => self.baseline,
        }
    }
    fn set(&mut self, name: &str, value: &str) {
        match (name, value) {
            ("direction", "ltr") => self.direction = "ltr",
            ("direction", "rtl") => self.direction = "rtl",
            ("direction", "inherit") => self.direction = "inherit",
            ("textAlign", "left") => self.align = "left",
            ("textAlign", "right") => self.align = "right",
            ("textAlign", "center") => self.align = "center",
            ("textAlign", "start") => self.align = "start",
            ("textAlign", "end") => self.align = "end",
            ("textBaseline", "top") => self.baseline = "top",
            ("textBaseline", "hanging") => self.baseline = "hanging",
            ("textBaseline", "middle") => self.baseline = "middle",
            ("textBaseline", "alphabetic") => self.baseline = "alphabetic",
            ("textBaseline", "ideographic") => self.baseline = "ideographic",
            ("textBaseline", "bottom") => self.baseline = "bottom",
            _ => {}
        }
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
    for name in ["direction", "textAlign", "textBaseline"] {
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
fn get(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Ok(canvas) = canvas_receiver_node(scope, &state, &args) else {
        return;
    };
    let name = args.data().to_rust_string_lossy(scope);
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
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Ok(canvas) = canvas_receiver_node(scope, &state, &args) else {
        return;
    };
    let name = args.data().to_rust_string_lossy(scope);
    // ToString can run author code, resize/reset the surface or throw. Never
    // retain a draw-state borrow over that conversion; revalidate afterwards.
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    let Ok(current) = canvas_receiver_node(scope, &state, &args) else {
        return;
    };
    if current != canvas {
        return;
    }
    state
        .canvas_draw_states
        .borrow_mut()
        .entry(canvas)
        .or_default()
        .text
        .set(&name, &value);
}

#[cfg(test)]
mod tests {
    use super::*;
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
