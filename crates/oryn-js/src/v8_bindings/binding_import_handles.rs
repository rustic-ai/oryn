pub struct BindingNativeDeclarationView {
    pub view: BindingDeclarationView,
    pub allocation: std::sync::Arc<dyn Send + Sync>,
}
impl std::ops::Deref for BindingNativeDeclarationView {
    type Target = BindingDeclarationView;
    fn deref(&self) -> &Self::Target {
        &self.view
    }
}
#[derive(Debug)]
pub enum BindingNativeCssError {
    Security,
    Native(String),
}
impl From<String> for BindingNativeCssError {
    fn from(value: String) -> Self {
        Self::Native(value)
    }
}
impl From<&str> for BindingNativeCssError {
    fn from(value: &str) -> Self {
        Self::Native(value.into())
    }
}
fn native_css_read_error(interface: &str, error: BindingNativeCssError) -> V8BindingError {
    match error {
        BindingNativeCssError::Security => V8BindingError::Unsupported {
            interface: interface.into(),
            member: "SecurityError".into(),
        },
        BindingNativeCssError::Native(message) => V8BindingError::EventCallback(message),
    }
}
// Dependency-neutral native CSSOM handles, added to oryn-js bindings. These
// contain no Stylo types and no authored JavaScript implementation.
pub trait BindingNativeStylesheet {
    fn title(&self) -> Result<Option<String>, BindingNativeCssError>;
    fn disabled(&self) -> Result<bool, BindingNativeCssError>;
    fn set_disabled(&self, disabled: bool) -> Result<bool, BindingNativeCssError>;
    fn media_text(&self) -> Result<String, BindingNativeCssError>;
    fn media_count(&self) -> Result<usize, BindingNativeCssError>;
    fn media_at(&self, index: usize) -> Result<Option<String>, BindingNativeCssError>;
    fn edit_media(
        &self,
        edit: BindingMediaListEdit<'_>,
    ) -> Result<BindingStylesheetMediaMutation, BindingNativeCssError>;
    fn rule_count(&self) -> Result<usize, BindingNativeCssError>;
    fn rules(&self) -> Result<Vec<BindingRuleView>, BindingNativeCssError>;
    fn href(&self) -> Option<String>;
    fn owner_node(&self) -> Result<Option<NodeId>, BindingNativeCssError>;
    fn origin_clean(&self) -> bool;
    fn parent(&self) -> Result<Option<Rc<dyn BindingNativeStylesheet>>, BindingNativeCssError>;
    fn identity(&self) -> u64;
    fn insert_rule(&self, index: usize, source: &str) -> Result<(), BindingNativeRuleError>;
    fn delete_rule(&self, index: usize) -> Result<(), BindingNativeRuleError>;
    fn import_at(
        &self,
        index: usize,
    ) -> Result<Option<Rc<dyn BindingNativeImport>>, BindingNativeCssError>;
    fn rule_at(
        &self,
        index: usize,
    ) -> Result<Option<Rc<dyn BindingNativeRule>>, BindingNativeCssError>;
}
pub trait BindingNativeRule {
    fn layer_name_count(&self) -> Result<Option<usize>, BindingNativeCssError>;
    fn layer_name_at(&self, index: usize) -> Result<Option<String>, BindingNativeCssError>;
    fn identity(&self) -> u64;
    fn as_import(&self) -> Result<Option<Rc<dyn BindingNativeImport>>, BindingNativeCssError>;
    fn view(&self) -> Result<BindingRuleView, BindingNativeCssError>;
    fn sheet(&self) -> Result<Option<Rc<dyn BindingNativeStylesheet>>, BindingNativeCssError>;
    fn parent_rule(&self) -> Result<Option<Rc<dyn BindingNativeRule>>, BindingNativeCssError>;
    fn child_count(&self) -> Result<usize, BindingNativeCssError>;
    fn child_at(
        &self,
        index: usize,
    ) -> Result<Option<Rc<dyn BindingNativeRule>>, BindingNativeCssError>;
    fn set_selector_text(&self, text: &str) -> Result<bool, BindingNativeCssError>;
    fn declaration(
        &self,
        property: &str,
    ) -> Result<BindingNativeDeclarationView, BindingNativeCssError>;
    fn set_declaration_text(&self, source: &str) -> Result<(), BindingNativeCssError>;
    fn set_property(
        &self,
        property: &str,
        value: &str,
        priority: &str,
    ) -> Result<BindingDeclarationMutation, BindingNativeCssError>;
    fn insert_rule(&self, index: usize, source: &str) -> Result<(), BindingNativeRuleError>;
    fn delete_rule(&self, index: usize) -> Result<(), BindingNativeRuleError>;
}
pub trait BindingNativeImport {
    fn media_count(&self) -> Result<usize, BindingNativeCssError>;
    fn media_at(&self, index: usize) -> Result<Option<String>, BindingNativeCssError>;
    fn identity(&self) -> u64;
    fn href(&self) -> Result<String, BindingNativeCssError>;
    fn layer_name(&self) -> Result<Option<String>, BindingNativeCssError>;
    fn supports_text(&self) -> Result<Option<String>, BindingNativeCssError>;
    fn media_text(&self) -> Result<String, BindingNativeCssError>;
    fn media_queries(&self) -> Result<Vec<String>, BindingNativeCssError>;
    fn edit_media(
        &self,
        edit: BindingMediaListEdit<'_>,
    ) -> Result<BindingStylesheetMediaMutation, BindingNativeCssError>;
    fn sheet(&self) -> Result<Option<Rc<dyn BindingNativeStylesheet>>, BindingNativeCssError>;
    fn parent(&self) -> Result<Option<Rc<dyn BindingNativeStylesheet>>, BindingNativeCssError>;
}
pub enum BindingNativeRuleError {
    IndexSize,
    HierarchyRequest,
    Syntax,
    InvalidState,
    Security,
    Native(String),
}

/// Weak object cache does not retain a detached native graph. Checked V8
/// private edges sheet→rule, rule→child, child→parent keep reachable wrappers
/// and their native records alive; deletion removes only the parent edge.
#[derive(Default)]
pub(super) struct NativeStylesheetWrappers {
    sheets: BTreeMap<u64, v8::Weak<v8::Object>>,
    imports: BTreeMap<u64, v8::Weak<v8::Object>>,
    records: BTreeMap<u64, Rc<dyn BindingNativeStylesheet>>,
    import_records: BTreeMap<u64, Rc<dyn BindingNativeImport>>,
    rules: BTreeMap<u64, v8::Weak<v8::Object>>,
    rule_records: BTreeMap<u64, Rc<dyn BindingNativeRule>>,
}

fn native_css_identity(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    label: &str,
) -> Option<u64> {
    if object.is_proxy() {
        return None;
    }
    let name = v8::String::new(scope, label)?;
    let key = v8::Private::for_api(scope, Some(name));
    let value = object.get_private(scope, key)?;
    let integer = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (id, exact) = integer.u64_value();
    exact.then_some(id)
}

fn private_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    label: &str,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if object.is_proxy() {
        return Err(V8BindingError::Unsupported {
            interface: "CSSRule".into(),
            member: "incompatible receiver".into(),
        });
    }
    let name = v8::String::new(scope, label)
        .ok_or(V8BindingError::Allocation("native CSS private key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    let value = object
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    v8::Local::<v8::Object>::try_from(value).map_err(|_| V8BindingError::Unsupported {
        interface: "CSSRule".into(),
        member: "incompatible receiver".into(),
    })
}

fn checked_native_css_edge(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    label: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, label)
        .ok_or(V8BindingError::Allocation("native CSS ownership edge"))?;
    let key = v8::Private::for_api(scope, Some(name));
    if object.set_private(scope, key, value) != Some(true) {
        return Err(V8BindingError::Allocation("native CSS ownership edge"));
    }
    Ok(())
}

fn clear_native_css_edge(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    label: &str,
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, label)
        .ok_or(V8BindingError::Allocation("native CSS ownership edge"))?;
    let key = v8::Private::for_api(scope, Some(name));
    if object.delete_private(scope, key) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(())
}

/// Native identity keys are private BigInts; internal fields below select the
/// owning registry independently of the calling realm, including retired realms.
fn bind_native_css_owner(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    if object.internal_field_count() < 2 {
        return Err(V8BindingError::Allocation("native CSS owner fields"));
    }
    object.set_internal_field(0, v8::Integer::new(scope, 0).into());
    object.set_internal_field(1, v8::BigInt::new_from_u64(scope, state.instance_id).into());
    Ok(())
}

fn wrap_native_stylesheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    handle: Rc<dyn BindingNativeStylesheet>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let identity = handle.identity();
    let existing = state
        .native_stylesheets
        .borrow()
        .sheets
        .get(&identity)
        .and_then(|wrapper| wrapper.to_local(scope));
    if let Some(object) = existing {
        return Ok(object);
    }
    let template = state
        .instance_templates
        .borrow()
        .get("CSSStyleSheet")
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::MissingInterface("CSSStyleSheet"))?;
    let object = v8::Local::new(scope, template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("native stylesheet wrapper"))?;
    bind_native_css_owner(scope, state, object)?;
    let identity_value = v8::BigInt::new_from_u64(scope, identity);
    checked_native_css_edge(
        scope,
        object,
        "oryn.native.css.sheet",
        identity_value.into(),
    )?;
    // Provider callback is made with no mutable wrapper-map borrow held.
    let parent = handle
        .parent()
        .map_err(|error| native_css_read_error("CSSStyleSheet", error))?;
    if let Some(parent) = parent {
        let parent = wrap_native_stylesheet(scope, state, parent)?;
        checked_native_css_edge(scope, object, "oryn.native.css.parent", parent.into())?;
    }
    let weak_state = Rc::downgrade(state);
    let weak = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            if let Some(state) = weak_state.upgrade() {
                let removed = {
                    let mut wrappers = state.native_stylesheets.borrow_mut();
                    let weak = wrappers.sheets.remove(&identity);
                    let record = wrappers.records.remove(&identity);
                    (weak, record)
                };
                drop(removed);
            }
        }),
    );
    let mut wrappers = state.native_stylesheets.borrow_mut();
    wrappers.records.insert(identity, handle);
    wrappers.sheets.insert(identity, weak);
    Ok(object)
}

fn native_import_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let invalid = || V8BindingError::Unsupported {
        interface: "CSSImportRule".into(),
        member: "incompatible receiver".into(),
    };
    let identity =
        native_css_identity(scope, object, "oryn.native.css.import").ok_or_else(invalid)?;
    let handle = state
        .native_stylesheets
        .borrow()
        .import_records
        .get(&identity)
        .cloned()
        .ok_or_else(invalid)?;
    let failure = |error| native_css_read_error("CSSImportRule", error);
    match member {
        "href" => v8_string(scope, &handle.href().map_err(failure)?),
        "layerName" | "supportsText" => {
            let text = if member == "layerName" {
                handle.layer_name()
            } else {
                handle.supports_text()
            }
            .map_err(failure)?;
            text.map_or(Ok(v8::null(scope).into()), |text| v8_string(scope, &text))
        }
        "styleSheet" | "parentStyleSheet" => {
            let sheet = if member == "styleSheet" {
                handle.sheet()
            } else {
                handle.parent()
            }
            .map_err(failure)?;
            let Some(sheet) = sheet else {
                clear_native_css_edge(
                    scope,
                    object,
                    if member == "styleSheet" {
                        "oryn.native.css.child"
                    } else {
                        "oryn.native.css.parent"
                    },
                )?;
                return Ok(v8::null(scope).into());
            };
            let sheet = wrap_native_stylesheet(scope, state, sheet)?;
            if member == "styleSheet" {
                checked_native_css_edge(scope, object, "oryn.native.css.child", sheet.into())?;
            }
            Ok(sheet.into())
        }
        "media" => Ok(wrap_native_import_media(scope, state, object)?.into()),
        _ => Err(V8BindingError::Unsupported {
            interface: "CSSImportRule".into(),
            member: member.into(),
        }),
    }
}

fn wrap_native_import<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    handle: Rc<dyn BindingNativeImport>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let identity = handle.identity();
    let existing = state
        .native_stylesheets
        .borrow()
        .imports
        .get(&identity)
        .and_then(|wrapper| wrapper.to_local(scope));
    if let Some(object) = existing {
        return Ok(object);
    }
    let template = state
        .instance_templates
        .borrow()
        .get("CSSImportRule")
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::MissingInterface("CSSImportRule"))?;
    let object = v8::Local::new(scope, template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("native CSSImportRule"))?;
    bind_native_css_owner(scope, state, object)?;
    let value = v8::BigInt::new_from_u64(scope, identity);
    checked_native_css_edge(scope, object, "oryn.native.css.import", value.into())?;
    let parent = handle
        .parent()
        .map_err(|error| native_css_read_error("CSSImportRule", error))?;
    if let Some(parent) = parent {
        let parent = wrap_native_stylesheet(scope, state, parent)?;
        checked_native_css_edge(scope, object, "oryn.native.css.parent", parent.into())?;
    }
    let weak_state = Rc::downgrade(state);
    let wrapper = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_| {
            if let Some(state) = weak_state.upgrade() {
                let removed = {
                    let mut wrappers = state.native_stylesheets.borrow_mut();
                    let weak = wrappers.imports.remove(&identity);
                    let record = wrappers.import_records.remove(&identity);
                    (weak, record)
                };
                drop(removed);
            }
        }),
    );
    let mut wrappers = state.native_stylesheets.borrow_mut();
    wrappers.import_records.insert(identity, handle);
    wrappers.imports.insert(identity, wrapper);
    Ok(object)
}

fn wrap_native_import_media<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    rule: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if let Ok(list) = private_object(scope, rule, "oryn.native.css.media") {
        return Ok(list);
    }
    let template = state
        .instance_templates
        .borrow()
        .get("MediaList")
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::MissingInterface("MediaList"))?;
    let list = v8::Local::new(scope, template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("native import MediaList"))?;
    bind_native_css_owner(scope, state, list)?;
    checked_native_css_edge(scope, list, "oryn.native.css.media.rule", rule.into())?;
    checked_native_css_edge(scope, rule, "oryn.native.css.media", list.into())?;
    Ok(list)
}

fn native_import_media_handle(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
) -> Result<Rc<dyn BindingNativeImport>, V8BindingError> {
    let rule = private_object(scope, object, "oryn.native.css.media.rule")?;
    let identity = native_css_identity(scope, rule, "oryn.native.css.import")
        .ok_or(V8BindingError::Allocation("native import media receiver"))?;
    state
        .native_stylesheets
        .borrow()
        .import_records
        .get(&identity)
        .cloned()
        .ok_or(V8BindingError::Allocation("native import media owner"))
}
