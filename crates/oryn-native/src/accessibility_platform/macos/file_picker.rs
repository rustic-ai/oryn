//! AppKit-owned asynchronous file panels. Paths exist only in the UI/parent
//! process; the page worker receives selected bytes after policy and lifetime checks.
use super::*;
struct Pending {
    window: Id,
    panel: Id,
    delegate: Id,
    request: ActionRequest,
    sender: SyncSender<ActionRequest>,
    live: bool,
}
thread_local! {
 static PENDING:RefCell<BTreeMap<usize,Pending>>=RefCell::new(BTreeMap::new());
 static SHOWN:RefCell<BTreeMap<(usize,SemanticRef,u64),oryn_dom::FilePicker>>=RefCell::new(BTreeMap::new());
 static FAILURE:RefCell<Option<String>>=RefCell::new(None);
}
unsafe extern "C" fn ended(delegate: Id, _: Sel, panel: Id, code: isize, _: *mut c_void) {
    unsafe {
        let Some(mut pending) =
            PENDING.with(|pending| pending.borrow_mut().remove(&(delegate as usize)))
        else {
            return;
        };
        if pending.live {
            let result = (|| -> Result<Option<Vec<std::path::PathBuf>>, String> {
                if code != 1 {
                    return Ok(None);
                }
                let urls = object(panel, "URLs");
                let count: unsafe extern "C" fn(Id, Sel) -> usize =
                    std::mem::transmute(objc_msgSend as *const ());
                let count = count(urls, sel("count"));
                if count == 0 {
                    return Ok(None);
                }
                if count > 128 {
                    return Err("native file chooser count exceeds limit".into());
                }
                let at: unsafe extern "C" fn(Id, Sel, usize) -> Id =
                    std::mem::transmute(objc_msgSend as *const ());
                let mut paths = Vec::with_capacity(count);
                for index in 0..count {
                    let path = rust_string(object(at(urls, sel("objectAtIndex:"), index), "path"))
                        .ok_or("native file chooser path unavailable")?;
                    if path.len() > 32 * 1024 {
                        return Err("native file chooser path exceeds limit".into());
                    }
                    paths.push(std::path::PathBuf::from(path));
                }
                Ok(Some(paths))
            })();
            match result {
                Ok(paths) => {
                    if let Action::FileSelection {
                        paths: selected, ..
                    } = &mut pending.request.action
                    {
                        *selected = paths;
                    }
                    if pending.sender.try_send(pending.request).is_err() {
                        FAILURE.with(|failure| {
                            *failure.borrow_mut() =
                                Some("native file chooser completion queue unavailable".into())
                        });
                    }
                }
                Err(error) => FAILURE.with(|failure| *failure.borrow_mut() = Some(error)),
            }
        }
        void_arg(pending.panel, "orderOut:", std::ptr::null_mut());
        void(pending.panel, "release");
        void(pending.delegate, "release");
    }
}
unsafe fn delegate_class() -> Result<Id, String> {
    unsafe {
        let existing = class("OrynFilePickerDelegate");
        if !existing.is_null() {
            return Ok(existing);
        }
        let name = CString::new("OrynFilePickerDelegate").unwrap();
        let kind = objc_allocateClassPair(class("NSObject"), name.as_ptr(), 0);
        if kind.is_null() {
            return Err("native file chooser delegate allocation failed".into());
        }
        let encoding = CString::new("v@:@q^v").unwrap();
        if !class_addMethod(
            kind,
            sel("panelDidEnd:returnCode:contextInfo:"),
            ended as *const c_void,
            encoding.as_ptr(),
        ) {
            return Err("native file chooser callback installation failed".into());
        }
        objc_registerClassPair(kind);
        Ok(kind)
    }
}
pub(super) unsafe fn publish(
    window: Id,
    sender: &SyncSender<ActionRequest>,
    snapshot: &Snapshot,
) -> Result<(), String> {
    unsafe {
        if let Some(error) = FAILURE.with(|failure| failure.borrow_mut().take()) {
            return Err(error);
        }
        SHOWN.with(|shown| {
            shown.borrow_mut().retain(|(owner, target, _), expected| {
                *owner != window as usize || snapshot.file_pickers.get(target) == Some(expected)
            })
        });
        let cancel = PENDING.with(|pending| {
            let mut pending = pending.borrow_mut();
            let mut panels = Vec::new();
            for value in pending
                .values_mut()
                .filter(|pending| pending.window == window && pending.live)
            {
                let valid = if let Action::FileSelection { expected, .. } = &value.request.action {
                    snapshot.file_pickers.get(&value.request.target) == Some(expected)
                } else {
                    false
                };
                if !valid {
                    value.live = false;
                    panels.push(value.panel)
                }
            }
            panels
        });
        for panel in cancel {
            void_arg(panel, "cancel:", std::ptr::null_mut());
        }
        if PENDING.with(|pending| {
            pending
                .borrow()
                .values()
                .any(|pending| pending.window == window)
        }) {
            return Ok(());
        }
        let next = snapshot.file_pickers.iter().find(|(owner, picker)| {
            !SHOWN.with(|shown| {
                shown
                    .borrow()
                    .contains_key(&(window as usize, **owner, picker.session))
            })
        });
        let Some((owner, picker)) = next else {
            return Ok(());
        };
        let node = snapshot
            .nodes
            .get(owner)
            .ok_or("native file chooser owner missing")?;
        let delegate_type = delegate_class()?;
        let panel = object(class("NSOpenPanel"), "openPanel");
        if panel.is_null() {
            return Err("native file chooser unavailable".into());
        }
        object(panel, "retain");
        let delegate = object(object(delegate_type, "alloc"), "init");
        if delegate.is_null() {
            void(panel, "release");
            return Err("native file chooser delegate unavailable".into());
        }
        let set_bool: unsafe extern "C" fn(Id, Sel, i8) =
            std::mem::transmute(objc_msgSend as *const ());
        set_bool(panel, sel("setCanChooseFiles:"), 1);
        set_bool(panel, sel("setCanChooseDirectories:"), 0);
        set_bool(
            panel,
            sel("setAllowsMultipleSelection:"),
            picker.multiple as i8,
        );
        // Extensions are hints; MIME-only filters remain unrestricted because AppKit
        // must not guess a narrower format from an unrecognized media type.
        let extensions = picker
            .accept
            .split(',')
            .map(str::trim)
            .filter_map(|token| token.strip_prefix('.'))
            .filter(|token| {
                !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            .map(str::to_string)
            .collect::<Vec<_>>();
        if !extensions.is_empty()
            && picker
                .accept
                .split(',')
                .all(|token| token.trim().starts_with('.'))
        {
            let borrowed = extensions.iter().map(String::as_str).collect::<Vec<_>>();
            void_arg(panel, "setAllowedFileTypes:", strings(&borrowed));
            set_bool(panel, sel("setAllowsOtherFileTypes:"), 1);
        }
        let request = ActionRequest {
            scope: node.scope.clone(),
            target: *owner,
            revision: node.revision,
            action: Action::FileSelection {
                expected: picker.clone(),
                paths: None,
            },
        };
        PENDING.with(|pending| {
            pending.borrow_mut().insert(
                delegate as usize,
                Pending {
                    window,
                    panel,
                    delegate,
                    request,
                    sender: sender.clone(),
                    live: true,
                },
            )
        });
        SHOWN.with(|shown| {
            shown
                .borrow_mut()
                .insert((window as usize, *owner, picker.session), picker.clone())
        });
        let begin: unsafe extern "C" fn(Id, Sel, Id, Id, Id, Id, Id, Sel, *mut c_void) =
            std::mem::transmute(objc_msgSend as *const ());
        begin(
            panel,
            sel(
                "beginSheetForDirectory:file:types:modalForWindow:modalDelegate:didEndSelector:contextInfo:",
            ),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            window,
            delegate,
            sel("panelDidEnd:returnCode:contextInfo:"),
            std::ptr::null_mut(),
        );
        Ok(())
    }
}
pub(super) unsafe fn retire(window: Id) {
    unsafe {
        let panels = PENDING.with(|pending| {
            let mut pending = pending.borrow_mut();
            pending
                .values_mut()
                .filter_map(|pending| {
                    if pending.window == window {
                        pending.live = false;
                        Some(pending.panel)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        });
        for panel in panels {
            void_arg(panel, "cancel:", std::ptr::null_mut());
        }
        SHOWN.with(|shown| {
            shown
                .borrow_mut()
                .retain(|(owner, _, _), _| *owner != window as usize)
        });
    }
}
