// Native retained lists: length/item query the original Stylo list, not a
// reparsed snapshot or a freshly materialized Vec of all descendants.
enum NativeRuleListOwner {
    Sheet(Rc<dyn BindingNativeStylesheet>),
    Group(Rc<dyn BindingNativeRule>),
}
impl NativeRuleListOwner {
    fn count(&self) -> Result<usize, BindingNativeCssError> {
        match self {
            Self::Sheet(sheet) => sheet.rule_count(),
            Self::Group(rule) => rule.child_count(),
        }
    }
    fn at(&self, index: usize) -> Result<Option<Rc<dyn BindingNativeRule>>, BindingNativeCssError> {
        match self {
            Self::Sheet(sheet) => sheet.rule_at(index),
            Self::Group(rule) => rule.child_at(index),
        }
    }
}

/// Derive the owner from private traced wrapper edges on every operation.
/// No Rust Global roots or independent snapshot arrays retain this list.
fn native_rule_list_owner(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
) -> Option<NativeRuleListOwner> {
    if object.is_proxy() {
        return None;
    }
    for (label, is_sheet) in [
        ("oryn.native.css.list.sheet", true),
        ("oryn.native.css.list.group", false),
    ] {
        let name = v8::String::new(scope, label)?;
        let key = v8::Private::for_api(scope, Some(name));
        let value = object.get_private(scope, key)?;
        let Ok(owner) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        let identity = native_css_identity(
            scope,
            owner,
            if is_sheet {
                "oryn.native.css.sheet"
            } else {
                "oryn.native.css.rule"
            },
        )?;
        let wrappers = state.native_stylesheets.borrow();
        return if is_sheet {
            wrappers
                .records
                .get(&identity)
                .cloned()
                .map(NativeRuleListOwner::Sheet)
        } else {
            wrappers
                .rule_records
                .get(&identity)
                .cloned()
                .map(NativeRuleListOwner::Group)
        };
    }
    None
}

fn native_rule_list_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
) -> Option<Result<v8::Local<'s, v8::Value>, V8BindingError>> {
    let owner = native_rule_list_owner(scope, state, object)?;
    Some((|| {
        let count = owner
            .count()
            .map_err(|error| native_css_read_error("CSSRuleList", error))?;
        let count = u32::try_from(count)
            .map_err(|_| V8BindingError::Allocation("CSSRuleList length overflow"))?;
        Ok(v8::Integer::new_from_unsigned(scope, count).into())
    })())
}

/// item()'s caller performs ordinary WebIDL unsigned-long conversion; indexed
/// getter uses the same operation but returns undefined for a missing index.
fn native_rule_list_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    index: u32,
    indexed: bool,
) -> Option<Result<v8::Local<'s, v8::Value>, V8BindingError>> {
    let owner = native_rule_list_owner(scope, state, object)?;
    Some((|| {
        let Some(handle) = owner
            .at(index as usize)
            .map_err(|error| native_css_read_error("CSSRuleList", error))?
        else {
            return Ok(if indexed {
                v8::undefined(scope).into()
            } else {
                v8::null(scope).into()
            });
        };
        Ok(wrap_native_rule(scope, state, handle)?.into())
    })())
}

fn wrap_native_rule_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    owner: v8::Local<v8::Object>,
    sheet: bool,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let name = v8::String::new(scope, "oryn.native.css.list")
        .ok_or(V8BindingError::Allocation("native CSS rule list key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    if let Some(value) = owner.get_private(scope, key)
        && let Ok(list) = v8::Local::<v8::Object>::try_from(value)
    {
        return Ok(list);
    }
    let template = state
        .instance_templates
        .borrow()
        .get("CSSRuleList")
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::MissingInterface("CSSRuleList"))?;
    let list = v8::Local::new(scope, template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("native CSS rule list"))?;
    bind_native_css_owner(scope, state, list)?;
    checked_native_css_edge(
        scope,
        list,
        if sheet {
            "oryn.native.css.list.sheet"
        } else {
            "oryn.native.css.list.group"
        },
        owner.into(),
    )?;
    checked_native_css_edge(scope, owner, "oryn.native.css.list", list.into())?;
    Ok(list)
}
