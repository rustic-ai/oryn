// Consumers for the typed v18 binding handle API. These are invoked before
// legacy page-rule MediaList dispatch only when the native private owner exists.
enum NativeMediaOwner {
    Sheet(Rc<dyn BindingNativeStylesheet>),
    Import(Rc<dyn BindingNativeImport>),
}
impl NativeMediaOwner {
    fn text(&self) -> Result<String, BindingNativeCssError> {
        match self {
            Self::Sheet(value) => value.media_text(),
            Self::Import(value) => value.media_text(),
        }
    }
    fn count(&self) -> Result<usize, BindingNativeCssError> {
        match self {
            Self::Sheet(value) => value.media_count(),
            Self::Import(value) => value.media_count(),
        }
    }
    fn at(&self, index: usize) -> Result<Option<String>, BindingNativeCssError> {
        match self {
            Self::Sheet(value) => value.media_at(index),
            Self::Import(value) => value.media_at(index),
        }
    }
    fn edit(
        &self,
        edit: BindingMediaListEdit<'_>,
    ) -> Result<BindingStylesheetMediaMutation, BindingNativeCssError> {
        match self {
            Self::Sheet(value) => value.edit_media(edit),
            Self::Import(value) => value.edit_media(edit),
        }
    }
}
fn native_media_owner(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
) -> Option<Result<NativeMediaOwner, V8BindingError>> {
    if let Ok(sheet) = private_object(scope, object, "oryn.native.css.media.sheet") {
        return Some(
            native_sheet_handle(scope, state, sheet)
                .map(NativeMediaOwner::Sheet)
                .ok_or_else(|| V8BindingError::Unsupported {
                    interface: "MediaList".into(),
                    member: "incompatible receiver".into(),
                }),
        );
    }
    private_object(scope, object, "oryn.native.css.media.rule").ok()?;
    Some(native_import_media_handle(scope, state, object).map(NativeMediaOwner::Import))
}
fn native_media_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Option<Result<v8::Local<'s, v8::Value>, V8BindingError>> {
    let handle = match native_media_owner(scope, state, object)? {
        Ok(handle) => handle,
        Err(error) => return Some(Err(error)),
    };
    Some((|| match member {
        "mediaText" => v8_string(
            scope,
            &handle
                .text()
                .map_err(|error| native_css_read_error("MediaList", error))?,
        ),
        "length" => {
            let count = handle
                .count()
                .map_err(|error| native_css_read_error("MediaList", error))?;
            let length = u32::try_from(count)
                .map_err(|_| V8BindingError::Allocation("native MediaList length overflow"))?;
            Ok(v8::Integer::new_from_unsigned(scope, length).into())
        }
        _ => Err(V8BindingError::Unsupported {
            interface: "MediaList".into(),
            member: member.into(),
        }),
    })())
}


fn native_media_set_text(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    input: v8::Local<v8::Value>,
) -> Option<Result<(), V8BindingError>> {
    let owner = match native_media_owner(scope, state, object)? {
        Ok(owner) => owner,
        Err(error) => return Some(Err(error)),
    };
    Some((|| {
        let text = cssom_string(scope, input, false)?;
        owner
            .edit(BindingMediaListEdit::Replace(&text))
            .map_err(|error| native_css_read_error("MediaList", error))?;
        Ok(())
    })())
}

fn native_media_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments<'s>,
    member: &str,
) -> Option<Result<v8::Local<'s, v8::Value>, V8BindingError>> {
    let owner = match native_media_owner(scope, state, args.this())? {
        Ok(owner) => owner,
        Err(error) => return Some(Err(error)),
    };
    Some((|| {
        if member == "item" {
            let index = args
                .get(0)
                .uint32_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            return match owner
                .at(index as usize)
                .map_err(|error| native_css_read_error("MediaList", error))?
            {
                Some(value) => v8_string(scope, &value),
                None => Ok(v8::null(scope).into()),
            };
        }
        let text = cssom_string(scope, args.get(0), false)?;
        let edit = match member {
            "appendMedium" => BindingMediaListEdit::Append(&text),
            "deleteMedium" => BindingMediaListEdit::Delete(&text),
            _ => {
                return Err(V8BindingError::Unsupported {
                    interface: "MediaList".into(),
                    member: member.into(),
                });
            }
        };
        let result = owner
            .edit(edit)
            .map_err(|error| native_css_read_error("MediaList", error))?;
        if result.not_found {
            return Err(V8BindingError::Unsupported {
                interface: "MediaList".into(),
                member: "NotFoundError".into(),
            });
        }
        Ok(v8::undefined(scope).into())
    })())
}
