//! AppKit accessibility objects. All Objective-C objects and callbacks stay on
//! the process main thread. The worker-facing boundary carries only Rust data.
use super::*;
use crate::owned_action_request::{NativeActionSender, OwnedActionRequest};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_void},
    marker::PhantomData,
    rc::Rc,
    sync::mpsc::{Receiver, SyncSender},
};
mod columns;
mod file_picker;
mod input;
mod picture_in_picture;
mod text_input;
mod text_markers;
pub enum Update {
    Snapshot(Box<Snapshot>),
    Receipts(Vec<crate::webdriver_actions::OsKeyReceipt>),
    #[doc(hidden)]
    OwnedReceipts(Vec<crate::webdriver_actions::OwnedOsKeyReceipt>),
}
type Id = *mut c_void;
type Sel = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    origin: Point,
    size: Size,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Range {
    location: usize,
    length: usize,
}
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_allocateClassPair(base: Id, name: *const c_char, extra: usize) -> Id;
    fn objc_registerClassPair(class: Id);
    fn class_addMethod(
        class: Id,
        selector: Sel,
        implementation: *const c_void,
        types: *const c_char,
    ) -> bool;
    fn objc_getProtocol(name: *const c_char) -> Id;
    fn class_addProtocol(class: Id, protocol: Id) -> bool;
    fn objc_msgSend();
}
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSAccessibilityPostNotification(element: Id, notification: Id);
    fn NSAccessibilityPostNotificationWithUserInfo(element: Id, notification: Id, info: Id);
    static NSAccessibilityAnnouncementRequestedNotification: Id;
    static NSAccessibilityAnnouncementKey: Id;
    static NSAccessibilityPriorityKey: Id;
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> Rect;
}
unsafe extern "C" {
    fn pthread_main_np() -> i32;
}
unsafe fn sel(name: &str) -> Sel {
    let name = CString::new(name).unwrap();
    unsafe { sel_registerName(name.as_ptr()) }
}
unsafe fn class(name: &str) -> Id {
    let name = CString::new(name).unwrap();
    unsafe { objc_getClass(name.as_ptr()) }
}
// Every send has the exact method ABI; object values are retained explicitly.
unsafe fn object(target: Id, name: &str) -> Id {
    let f: unsafe extern "C" fn(Id, Sel) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(target, sel(name)) }
}
unsafe fn void(target: Id, name: &str) {
    let f: unsafe extern "C" fn(Id, Sel) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(target, sel(name)) }
}
unsafe fn object_arg(target: Id, name: &str, arg: Id) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, Id) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(target, sel(name), arg) }
}
unsafe fn void_arg(target: Id, name: &str, arg: Id) {
    let f: unsafe extern "C" fn(Id, Sel, Id) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(target, sel(name), arg) }
}
unsafe fn string(value: &str) -> Id {
    let bytes = value.as_bytes();
    let f: unsafe extern "C" fn(Id, Sel, *const u8, usize, usize) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let allocated = unsafe { object(class("NSString"), "alloc") };
    let result = unsafe {
        f(
            allocated,
            sel("initWithBytes:length:encoding:"),
            bytes.as_ptr(),
            bytes.len(),
            4,
        )
    };
    unsafe { object(result, "autorelease") }
}
unsafe fn rust_string(value: Id) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let check: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    if unsafe { check(value, sel("isKindOfClass:"), class("NSString")) } == 0 {
        return None;
    }
    let f: unsafe extern "C" fn(Id, Sel) -> *const c_char =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let bytes = unsafe { f(value, sel("UTF8String")) };
    if bytes.is_null() {
        None
    } else {
        Some(
            unsafe { CStr::from_ptr(bytes) }
                .to_string_lossy()
                .into_owned(),
        )
    }
}
unsafe fn number(value: bool) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, i8) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(class("NSNumber"), sel("numberWithBool:"), value as i8) }
}
unsafe fn array(values: &[Id]) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, *const Id, usize) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe {
        f(
            class("NSArray"),
            sel("arrayWithObjects:count:"),
            values.as_ptr(),
            values.len(),
        )
    }
}
unsafe fn strings(values: &[&str]) -> Id {
    unsafe { array(&values.iter().map(|v| string(v)).collect::<Vec<_>>()) }
}
unsafe fn point(value: Point) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, Point) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(class("NSValue"), sel("valueWithPoint:"), value) }
}
unsafe fn size(value: Size) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, Size) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(class("NSValue"), sel("valueWithSize:"), value) }
}
unsafe fn utf16(value: &[u16]) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, *const u16, usize) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let result = unsafe {
        f(
            object(class("NSString"), "alloc"),
            sel("initWithCharacters:length:"),
            value.as_ptr(),
            value.len(),
        )
    };
    unsafe { object(result, "autorelease") }
}
unsafe fn range(value: Range) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, Range) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(class("NSValue"), sel("valueWithRange:"), value) }
}
unsafe fn get_range(value: Id) -> Option<Range> {
    if value.is_null() {
        return None;
    }
    let check: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    if unsafe { check(value, sel("isKindOfClass:"), class("NSValue")) } == 0 {
        return None;
    }
    let kind: unsafe extern "C" fn(Id, Sel) -> *const c_char =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let kind = unsafe { kind(value, sel("objCType")) };
    if kind.is_null()
        || !matches!(
            unsafe { CStr::from_ptr(kind) }.to_bytes(),
            b"{_NSRange=QQ}" | b"{NSRange=QQ}"
        )
    {
        return None;
    }
    let f: unsafe extern "C" fn(Id, Sel) -> Range =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    Some(unsafe { f(value, sel("rangeValue")) })
}
unsafe fn native_units(value: Id) -> Option<Vec<u16>> {
    if value.is_null() {
        return None;
    }
    let check: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    if unsafe { check(value, sel("isKindOfClass:"), class("NSString")) } == 0 {
        return None;
    }
    let length: unsafe extern "C" fn(Id, Sel) -> usize =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let length = unsafe { length(value, sel("length")) };
    if length > 1024 * 1024 {
        return None;
    }
    let mut units = vec![0; length];
    let get: unsafe extern "C" fn(Id, Sel, *mut u16, Range) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe {
        get(
            value,
            sel("getCharacters:range:"),
            units.as_mut_ptr(),
            Range {
                location: 0,
                length,
            },
        )
    };
    Some(units)
}
unsafe fn get_integer(value: Id) -> Option<usize> {
    if value.is_null() {
        return None;
    }
    let check: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    if unsafe { check(value, sel("isKindOfClass:"), class("NSNumber")) } == 0 {
        return None;
    }
    let real: unsafe extern "C" fn(Id, Sel) -> f64 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    let real = unsafe { real(value, sel("doubleValue")) };
    if !real.is_finite() || real < 0. || real.fract() != 0. {
        return None;
    }
    let get: unsafe extern "C" fn(Id, Sel) -> i64 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    usize::try_from(unsafe { get(value, sel("longLongValue")) }).ok()
}
unsafe fn rect(value: Rect) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, Rect) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(class("NSValue"), sel("valueWithRect:"), value) }
}
unsafe fn real(value: f64) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, f64) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(class("NSNumber"), sel("numberWithDouble:"), value) }
}
unsafe fn numeric_string(value: Id) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let check: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    if unsafe { check(value, sel("isKindOfClass:"), class("NSNumber")) } == 0 {
        return None;
    }
    let text = unsafe { rust_string(object(value, "stringValue")) }?;
    oryn_dom::parse_html_floating_point(&text).map(|_| text)
}
unsafe fn integer(value: usize) -> Id {
    let f: unsafe extern "C" fn(Id, Sel, usize) -> Id =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(class("NSNumber"), sel("numberWithUnsignedLongLong:"), value) }
}
fn role(value: &str) -> &'static str {
    super::role_mapping::role(value)
}
struct Entry {
    marker_namespace: [u8; 16],
    marker_scope: Option<crate::host::webdriver_script::AutomationRealmSnapshot>,
    table: Option<Rc<table::Table>>,
    table_objects: Rc<BTreeMap<SemanticRef, Id>>,
    node: Node,
    revision: Revision,
    parent: Id,
    children: Vec<Id>,
    relations: BTreeMap<AccessibilityRelation, Vec<Id>>,
    window: Id,
    screen: Option<Rect>,
    root_origin: Point,
    viewport_height: f64,
    sender: NativeActionSender,
}
thread_local! {static ENTRIES:RefCell<BTreeMap<usize,Entry>>=const { RefCell::new(BTreeMap::new()) };}
fn with_entry<T>(object: Id, default: T, f: impl FnOnce(&Entry) -> T) -> T {
    ENTRIES.with(|entries| {
        entries
            .try_borrow()
            .ok()
            .and_then(|entries| entries.get(&(object as usize)).map(f))
            .unwrap_or(default)
    })
}
fn virtual_focus_owner(object: Id) -> Option<Id> {
    ENTRIES.with(|entries| {
        let entries = entries.try_borrow().ok()?;
        let target = entries.get(&(object as usize))?;
        entries
            .iter()
            .find(|(_, owner)| {
                owner.window == target.window
                    && super::role_mapping::is_active_descendant(&owner.node, target.node.id)
            })
            .map(|(id, _)| *id as Id)
    })
}
fn projected_focused(object: Id, entry: &Entry) -> bool {
    virtual_focus_owner(object).is_some()
        || (entry.node.states.contains(&NodeState::Focused)
            && entry
                .relations
                .get(&AccessibilityRelation::ActiveDescendant)
                .is_none_or(Vec::is_empty))
}
fn enqueue(object: Id, action: Action) -> bool {
    let object = if matches!(&action, Action::Focus) {
        virtual_focus_owner(object).unwrap_or(object)
    } else {
        object
    };
    with_entry(object, false, |entry| {
        if entry.node.states.contains(&NodeState::Disabled) {
            return false;
        }
        let permitted = match &action {
            Action::Adjust { .. } => {
                matches!(entry.node.role.as_str(), "slider" | "spinbutton")
                    && entry.node.actions.contains(&SemanticAction::Type)
                    && !entry.node.states.contains(&NodeState::Disabled)
                    && !entry.node.states.contains(&NodeState::Readonly)
            }
            Action::FileSelection { .. } | Action::Input(_) | Action::CancelInput => false,
            Action::Press => entry.node.actions.contains(&SemanticAction::Click),
            Action::Focus => entry.node.actions.contains(&SemanticAction::Focus),
            Action::SelectText { .. } => {
                entry.node.text.as_ref().is_some_and(|text| !text.protected)
            }
            Action::SetValue(_) => {
                !entry.node.states.contains(&NodeState::Readonly)
                    && [
                        SemanticAction::Type,
                        SemanticAction::Check,
                        SemanticAction::Select,
                    ]
                    .iter()
                    .any(|action| entry.node.actions.contains(action))
            }
        };
        permitted
            && entry
                .sender
                .send(ActionRequest {
                    scope: entry.node.scope.clone(),
                    target: entry.node.id,
                    revision: entry.revision,
                    action,
                }, None)
                .is_ok()
    })
}
unsafe extern "C" fn ignored(object: Id, _: Sel) -> i8 {
    with_entry(object, 1, |_| 0)
}
unsafe extern "C" fn element(object: Id, _: Sel) -> i8 {
    with_entry(object, 0, |_| 1)
}
unsafe extern "C" fn attribute_names(_: Id, _: Sel) -> Id {
    unsafe {
        strings(&[
            "AXValidationError",
            "AXHelp",
            "AXOrientation",
            "AXPlaceholderValue",
            "AXValueDescription",
            "AXElementBusy",
            "AXARIACurrent",
            "AXARIAColumnCount",
            "AXARIARowCount",
            "AXARIAColumnIndex",
            "AXARIARowIndex",
            "AXARIAPosInSet",
            "AXARIASetSize",
            "AXMinValue",
            "AXMaxValue",
            "AXRole",
            "AXSubrole",
            "AXRoleDescription",
            "AXTitle",
            "AXDescription",
            "AXValue",
            "AXParent",
            "AXChildren",
            "AXWindow",
            "AXTopLevelUIElement",
            "AXPosition",
            "AXSize",
            "AXEnabled",
            "AXFocused",
            "AXFocusedUIElement",
            "AXSelected",
            "AXExpanded",
            "AXRequired",
            "AXIdentifier",
            "AXLinkedUIElements",
            "AXServesAsTitleForUIElements",
            "AXTitleUIElement",
            "AXStartTextMarker",
            "AXEndTextMarker",
            "AXSelectedTextMarkerRange",
            "AXNumberOfCharacters",
            "AXSelectedText",
            "AXSelectedTextRange",
            "AXRows",
            "AXVisibleRows",
            "AXColumns",
            "AXVisibleColumns",
            "AXRowCount",
            "AXColumnCount",
            "AXRowIndexRange",
            "AXColumnIndexRange",
            "AXRowHeaderUIElements",
            "AXColumnHeaderUIElements",
        ])
    }
}
unsafe extern "C" fn attribute(object: Id, _: Sel, name: Id) -> Id {
    let Some(name) = (unsafe { rust_string(name) }) else {
        return std::ptr::null_mut();
    };
    with_entry(object, std::ptr::null_mut(), |entry| unsafe {
        if let Some(value) = text_markers::attribute(entry, &name) {
            return value;
        }
        match name.as_str() {
            "AXColumns" | "AXVisibleColumns" => {
                if entry
                    .table
                    .as_ref()
                    .is_some_and(|table| table.root == entry.node.id)
                {
                    columns::for_table(object, name == "AXVisibleColumns")
                } else {
                    std::ptr::null_mut()
                }
            }
            "AXRows" | "AXVisibleRows" => entry
                .table
                .as_ref()
                .filter(|table| table.root == entry.node.id)
                .map_or(std::ptr::null_mut(), |table| {
                    array(
                        &table
                            .rows
                            .iter()
                            .filter_map(|id| entry.table_objects.get(id).copied())
                            .collect::<Vec<_>>(),
                    )
                }),
            "AXRowCount" | "AXColumnCount" => entry
                .table
                .as_ref()
                .filter(|table| table.root == entry.node.id)
                .and_then(|table| {
                    if name == "AXRowCount" {
                        table.row_count
                    } else {
                        table.column_count
                    }
                })
                .map_or(std::ptr::null_mut(), |value| integer(value)),
            "AXRowIndexRange" | "AXColumnIndexRange" => entry
                .table
                .as_ref()
                .and_then(|table| table.cells.get(&entry.node.id))
                .map_or(std::ptr::null_mut(), |cell| {
                    range(if name == "AXRowIndexRange" {
                        Range {
                            location: cell.row,
                            length: cell.row_span,
                        }
                    } else {
                        Range {
                            location: cell.column,
                            length: cell.column_span,
                        }
                    })
                }),
            "AXRowHeaderUIElements" | "AXColumnHeaderUIElements" => {
                entry.table.as_ref().map_or(std::ptr::null_mut(), |table| {
                    let headers = table.headers_for(entry.node.id, name == "AXRowHeaderUIElements");
                    array(
                        &headers
                            .iter()
                            .filter_map(|id| entry.table_objects.get(id).copied())
                            .collect::<Vec<_>>(),
                    )
                })
            }
            "AXRole" => string(
                if entry.node.role == "button" && super::role_mapping::popup(&entry.node.properties)
                {
                    "AXPopUpButton"
                } else {
                    role(&entry.node.role)
                },
            ),
            "AXSubrole" => super::role_mapping::subrole(
                &entry.node.role,
                entry.node.text.as_ref().is_some_and(|text| text.protected),
            )
            .map_or(std::ptr::null_mut(), |value| string(value)),
            "AXRoleDescription" => string(
                super::role_mapping::authored_description(&entry.node.role, &entry.node.properties)
                    .unwrap_or_else(|| {
                        if entry.node.role == "button"
                            && super::role_mapping::popup(&entry.node.properties)
                        {
                            "pop up button"
                        } else {
                            super::role_mapping::description(&entry.node.role)
                        }
                    }),
            ),
            "AXTitle" | "AXDescription" => string(&entry.node.name),
            "AXHelp" => string(entry.node.description.as_deref().unwrap_or("")),
            "AXOrientation" => string(super::role_mapping::orientation(
                &entry.node.role,
                entry
                    .node
                    .properties
                    .get("orientation")
                    .and_then(serde_json::Value::as_str),
            )),
            "AXPlaceholderValue" => entry
                .node
                .properties
                .get("placeholder")
                .and_then(serde_json::Value::as_str)
                .map_or(std::ptr::null_mut(), |value| string(value)),
            "AXValidationError" => entry
                .node
                .properties
                .get("validation_error")
                .and_then(serde_json::Value::as_str)
                .map_or(std::ptr::null_mut(), |value| string(value)),
            "AXValueDescription" => super::role_mapping::value_description(
                &entry.node.role,
                entry.node.actions.contains(&SemanticAction::Focus),
                &entry.node.properties,
            )
            .map_or(std::ptr::null_mut(), |value| string(value)),
            "AXElementBusy" => number(
                entry
                    .node
                    .properties
                    .get("busy")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            ),
            "AXARIACurrent" => {
                let current = entry.node.properties.get("current");
                string(match current {
                    Some(serde_json::Value::Bool(true)) => "true",
                    Some(serde_json::Value::String(value))
                        if matches!(
                            value.as_str(),
                            "page" | "step" | "location" | "date" | "time"
                        ) =>
                    {
                        value
                    }
                    Some(serde_json::Value::String(value))
                        if !value.is_empty() && value != "false" =>
                    {
                        "true"
                    }
                    _ => "false",
                })
            }
            "AXARIAColumnCount" | "AXARIARowCount" | "AXARIAColumnIndex" | "AXARIARowIndex"
            | "AXARIAPosInSet" | "AXARIASetSize" => {
                let key = match name.as_str() {
                    "AXARIAColumnCount" => "colcount",
                    "AXARIARowCount" => "rowcount",
                    "AXARIAColumnIndex" => "colindex",
                    "AXARIARowIndex" => "rowindex",
                    "AXARIAPosInSet" => "posinset",
                    _ => "setsize",
                };
                entry
                    .node
                    .properties
                    .get(key)
                    .and_then(serde_json::Value::as_i64)
                    .or_else(|| {
                        entry
                            .table
                            .as_ref()
                            .and_then(|table| table.cells.get(&entry.node.id))
                            .and_then(|cell| match key {
                                "rowindex" => i64::try_from(cell.row + 1).ok(),
                                "colindex" => i64::try_from(cell.column + 1).ok(),
                                _ => None,
                            })
                    })
                    .map_or(std::ptr::null_mut(), |value| real(value as f64))
            }
            "AXNumberOfCharacters" => entry
                .node
                .text
                .as_ref()
                .filter(|text| !text.protected)
                .map_or(std::ptr::null_mut(), |text| integer(text.value.len())),
            "AXSelectedText" => entry
                .node
                .text
                .as_ref()
                .filter(|text| !text.protected)
                .and_then(|text| text.value.get(text.start as usize..text.end as usize))
                .map_or(std::ptr::null_mut(), |value| utf16(value)),
            "AXSelectedTextRange" => entry
                .node
                .text
                .as_ref()
                .filter(|text| !text.protected)
                .map_or(std::ptr::null_mut(), |text| {
                    range(Range {
                        location: text.start as usize,
                        length: text.end.saturating_sub(text.start) as usize,
                    })
                }),
            "AXMinValue" | "AXMaxValue" => super::role_mapping::numeric_bound(
                &entry.node.role,
                name == "AXMinValue",
                &entry.node.properties,
            )
            .map_or(std::ptr::null_mut(), |value| real(value)),
            "AXValue" => {
                if entry.node.role == "heading" {
                    entry
                        .node
                        .properties
                        .get("level")
                        .and_then(serde_json::Value::as_u64)
                        .map_or_else(|| integer(2), |value| integer(value as usize))
                } else if let Some(value) = entry
                    .node
                    .properties
                    .get("native_value_now")
                    .or_else(|| entry.node.properties.get("valuenow"))
                    .and_then(serde_json::Value::as_f64)
                {
                    real(value)
                } else if entry.node.text.as_ref().is_some_and(|text| text.protected) {
                    string("")
                } else if let Some(value) =
                    super::role_mapping::toggle_value(&entry.node.properties)
                {
                    integer(value)
                } else if entry.node.role == "tab" {
                    number(entry.node.states.contains(&NodeState::Selected))
                } else if let Some(text) = entry.node.text.as_ref().filter(|text| !text.protected) {
                    utf16(&text.value)
                } else if entry
                    .node
                    .properties
                    .get("checked")
                    .is_some_and(|value| value.as_str() == Some("mixed"))
                {
                    integer(2)
                } else if entry.node.states.contains(&NodeState::Checked) {
                    number(true)
                } else if matches!(entry.node.role.as_str(), "checkbox" | "radio") {
                    number(false)
                } else {
                    string(entry.node.value.as_deref().unwrap_or(""))
                }
            }
            "AXParent" => entry.parent,
            "AXChildren" => array(&entry.children),
            "AXWindow" | "AXTopLevelUIElement" => entry.window,
            "AXEnabled" => number(!entry.node.states.contains(&NodeState::Disabled)),
            "AXFocused" => number(projected_focused(object, entry)),
            "AXSelected" => number(entry.node.states.contains(&NodeState::Selected)),
            "AXExpanded" => number(entry.node.states.contains(&NodeState::Expanded)),
            "AXRequired" => number(entry.node.states.contains(&NodeState::Required)),
            "AXPosition" => entry
                .screen
                .map_or(std::ptr::null_mut(), |rect| point(rect.origin)),
            "AXSize" => entry
                .screen
                .map_or(std::ptr::null_mut(), |rect| size(rect.size)),
            "AXIdentifier" => string(&format!(
                "oryn:{}:{}:{}",
                entry.node.id.page_id.0, entry.node.id.document_generation, entry.node.id.node
            )),
            "AXFocusedUIElement" => entry
                .relations
                .get(&AccessibilityRelation::ActiveDescendant)
                .and_then(|targets| targets.first())
                .copied()
                .unwrap_or_else(|| {
                    if entry.node.states.contains(&NodeState::Focused) {
                        object
                    } else {
                        std::ptr::null_mut()
                    }
                }),
            "AXLinkedUIElements" => array(
                entry
                    .relations
                    .get(&AccessibilityRelation::Controls)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            ),
            "AXServesAsTitleForUIElements" => ENTRIES.with(|entries| {
                entries.try_borrow().ok().map_or_else(
                    || array(&[]),
                    |entries| {
                        let controls = super::role_mapping::labelled_controls(
                            entry.node.id,
                            entries
                                .values()
                                .filter(|candidate| candidate.window == entry.window)
                                .map(|entry| &entry.node),
                        );
                        array(
                            &controls
                                .into_iter()
                                .filter_map(|id| {
                                    entries
                                        .iter()
                                        .find(|(_, entry)| entry.node.id == id)
                                        .map(|(object, _)| *object as Id)
                                })
                                .collect::<Vec<_>>(),
                        )
                    },
                )
            }),
            "AXTitleUIElement" => entry
                .relations
                .get(&AccessibilityRelation::LabelledBy)
                .and_then(|v| v.first())
                .copied()
                .unwrap_or(std::ptr::null_mut()),
            _ => std::ptr::null_mut(),
        }
    })
}
unsafe extern "C" fn settable(object: Id, _: Sel, name: Id) -> i8 {
    let name = unsafe { rust_string(name) };
    with_entry(object, 0, |entry| match name.as_deref() {
        Some("AXFocused") => {
            (entry.node.actions.contains(&SemanticAction::Focus)
                || virtual_focus_owner(object).is_some()) as i8
        }
        Some("AXSelectedTextRange" | "AXSelectedTextMarkerRange") => {
            entry.node.text.as_ref().is_some_and(|text| !text.protected) as i8
        }
        Some("AXValue") => {
            (!entry.node.states.contains(&NodeState::Disabled)
                && !entry.node.states.contains(&NodeState::Readonly)
                && [
                    SemanticAction::Type,
                    SemanticAction::Check,
                    SemanticAction::Select,
                ]
                .iter()
                .any(|action| entry.node.actions.contains(action))) as i8
        }
        _ => 0,
    })
}
unsafe extern "C" fn set_attribute(object: Id, _: Sel, value: Id, name: Id) {
    match unsafe { rust_string(name) }.as_deref() {
        Some("AXSelectedTextMarkerRange") => {
            if let Some(action) = with_entry(object, None, |entry| unsafe {
                text_markers::selection(entry, value)
            }) {
                enqueue(object, action);
            }
        }
        Some("AXSelectedTextRange") => {
            if let Some(range) = unsafe { get_range(value) }
                && let Some(end) = range
                    .location
                    .checked_add(range.length)
                    .filter(|end| *end <= u32::MAX as usize)
            {
                let expected = with_entry(object, None, |entry| entry.node.text.clone());
                if let Some(expected) = expected {
                    enqueue(
                        object,
                        Action::SelectText {
                            start: range.location as u32,
                            end: end as u32,
                            expected,
                        },
                    );
                }
            }
        }
        Some("AXFocused") => {
            if unsafe { get_integer(value) }.is_some_and(|value| value != 0) {
                enqueue(object, Action::Focus);
            }
        }
        Some("AXValue") => {
            if let Some(text) = unsafe { native_units(value) }
                .map(|text| oryn_dom::dom_string::from_units(&text))
                .or_else(|| unsafe { numeric_string(value) })
            {
                enqueue(object, Action::SetValue(text));
            }
        }
        _ => {}
    }
}
unsafe extern "C" fn actions(object: Id, _: Sel) -> Id {
    with_entry(object, unsafe { array(&[]) }, |entry| unsafe {
        let mut names = Vec::new();
        if entry.node.actions.contains(&SemanticAction::Click) {
            names.push("AXPress");
            if super::role_mapping::offers_popup(&entry.node.role, &entry.node.properties) {
                names.push("AXShowMenu");
            }
        }
        if matches!(entry.node.role.as_str(), "slider" | "spinbutton")
            && entry.node.actions.contains(&SemanticAction::Type)
            && !entry.node.states.contains(&NodeState::Disabled)
            && !entry.node.states.contains(&NodeState::Readonly)
        {
            names.extend(["AXIncrement", "AXDecrement"]);
        }
        strings(&names)
    })
}
unsafe extern "C" fn perform(object: Id, _: Sel, name: Id) {
    match unsafe { rust_string(name) }.as_deref() {
        Some("AXPress" | "AXShowMenu") => {
            enqueue(object, Action::Press);
        }
        Some("AXIncrement") => {
            enqueue(object, Action::Adjust { increase: true });
        }
        Some("AXDecrement") => {
            enqueue(object, Action::Adjust { increase: false });
        }
        _ => {}
    }
}
unsafe extern "C" fn increment(object: Id, _: Sel) -> i8 {
    enqueue(object, Action::Adjust { increase: true }) as i8
}
unsafe extern "C" fn decrement(object: Id, _: Sel) -> i8 {
    enqueue(object, Action::Adjust { increase: false }) as i8
}
unsafe extern "C" fn press(object: Id, _: Sel) -> i8 {
    enqueue(object, Action::Press) as i8
}
fn depth(entries: &BTreeMap<usize, Entry>, entry: &Entry) -> usize {
    let mut parent = entry.parent as usize;
    let mut seen = BTreeSet::new();
    while seen.len() < 128 && seen.insert(parent) {
        let Some(entry) = entries.get(&parent) else {
            break;
        };
        parent = entry.parent as usize;
    }
    seen.len()
}
unsafe extern "C" fn focused(object: Id, _: Sel) -> Id {
    ENTRIES.with(|entries| {
        entries
            .try_borrow()
            .ok()
            .and_then(|entries| {
                entries
                    .iter()
                    .filter(|(_, entry)| {
                        entries
                            .get(&(object as usize))
                            .is_some_and(|owner| owner.window == entry.window)
                            && entry.node.states.contains(&NodeState::Focused)
                    })
                    .max_by_key(|(_, entry)| depth(&entries, entry))
                    .map(|(id, entry)| {
                        entry
                            .relations
                            .get(&AccessibilityRelation::ActiveDescendant)
                            .and_then(|targets| targets.first())
                            .copied()
                            .unwrap_or(*id as Id)
                    })
            })
            .unwrap_or(object)
    })
}
unsafe extern "C" fn hit_test(object: Id, _: Sel, point: Point) -> Id {
    ENTRIES.with(|entries| {
        entries
            .try_borrow()
            .ok()
            .and_then(|entries| {
                entries
                    .iter()
                    .filter(|(_, entry)| {
                        if !entries
                            .get(&(object as usize))
                            .is_some_and(|owner| owner.window == entry.window)
                        {
                            return false;
                        }
                        let in_bounds = entry.screen.is_some_and(|rect| {
                            point.x >= rect.origin.x
                                && point.y >= rect.origin.y
                                && point.x <= rect.origin.x + rect.size.width
                                && point.y <= rect.origin.y + rect.size.height
                        });
                        in_bounds
                            && entry.node.hit.as_ref().is_none_or(|hit| {
                                hit.contains(
                                    point.x - entry.root_origin.x,
                                    entry.viewport_height - (point.y - entry.root_origin.y),
                                )
                            })
                    })
                    .max_by(|(_, a), (_, b)| a.node.paint_order.cmp(&b.node.paint_order))
                    .map(|(id, _)| *id as Id)
            })
            .unwrap_or(object)
    })
}
unsafe extern "C" fn parameter_names(object: Id, _: Sel) -> Id {
    with_entry(object, unsafe { array(&[]) }, |entry| unsafe {
        let mut names = Vec::new();
        if entry.node.text.as_ref().is_some_and(|text| !text.protected) {
            names.extend([
                "AXStringForRange",
                "AXBoundsForRange",
                "AXLineForIndex",
                "AXRangeForLine",
            ]);
            names.extend_from_slice(text_markers::PARAMETERS);
        }
        if entry
            .table
            .as_ref()
            .is_some_and(|table| table.root == entry.node.id)
        {
            names.push("AXCellForColumnAndRow");
        }
        strings(&names)
    })
}
unsafe extern "C" fn parameter(object: Id, _: Sel, name: Id, value: Id) -> Id {
    let Some(name) = (unsafe { rust_string(name) }) else {
        return std::ptr::null_mut();
    };
    if name == "AXCellForColumnAndRow" {
        return with_entry(object, std::ptr::null_mut(), |entry| unsafe {
            let Some(table) = entry
                .table
                .as_ref()
                .filter(|table| table.root == entry.node.id)
            else {
                return std::ptr::null_mut();
            };
            let check: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
                std::mem::transmute(objc_msgSend as *const ());
            if value.is_null() || check(value, sel("isKindOfClass:"), class("NSArray")) == 0 {
                return std::ptr::null_mut();
            }
            let count: unsafe extern "C" fn(Id, Sel) -> usize =
                std::mem::transmute(objc_msgSend as *const ());
            if count(value, sel("count")) != 2 {
                return std::ptr::null_mut();
            }
            let at: unsafe extern "C" fn(Id, Sel, usize) -> Id =
                std::mem::transmute(objc_msgSend as *const ());
            let (Some(column), Some(row)) = (
                get_integer(at(value, sel("objectAtIndex:"), 0)),
                get_integer(at(value, sel("objectAtIndex:"), 1)),
            ) else {
                return std::ptr::null_mut();
            };
            table
                .cell_at(row, column)
                .and_then(|id| entry.table_objects.get(&id).copied())
                .unwrap_or(std::ptr::null_mut())
        });
    }
    with_entry(object, std::ptr::null_mut(), |entry| {
        if let Some(value) = unsafe { text_markers::parameter(object, entry, &name, value) } {
            return value;
        }
        let Some(text) = entry.node.text.as_ref().filter(|text| !text.protected) else {
            return std::ptr::null_mut();
        };
        unsafe {
            match name.as_str() {
                "AXStringForRange" => {
                    let Some(range) = get_range(value) else {
                        return std::ptr::null_mut();
                    };
                    range
                        .location
                        .checked_add(range.length)
                        .and_then(|end| text.value.get(range.location..end))
                        .map_or(std::ptr::null_mut(), |units| utf16(units))
                }
                "AXLineForIndex" => get_integer(value)
                    .and_then(|index| {
                        text.boxes
                            .iter()
                            .find(|item| item.start as usize <= index && index < item.end as usize)
                    })
                    .map_or(std::ptr::null_mut(), |item| integer(item.line)),
                "AXRangeForLine" => {
                    let Some(line) = get_integer(value) else {
                        return std::ptr::null_mut();
                    };
                    let boxes = text
                        .boxes
                        .iter()
                        .filter(|item| item.line == line)
                        .collect::<Vec<_>>();
                    let Some(start) = boxes.iter().map(|item| item.start).min() else {
                        return std::ptr::null_mut();
                    };
                    let end = boxes.iter().map(|item| item.end).max().unwrap_or(start);
                    range(Range {
                        location: start as usize,
                        length: (end - start) as usize,
                    })
                }
                "AXBoundsForRange" => {
                    let Some(range) = get_range(value) else {
                        return std::ptr::null_mut();
                    };
                    let Some(end) = range
                        .location
                        .checked_add(range.length)
                        .filter(|end| *end <= text.value.len())
                    else {
                        return std::ptr::null_mut();
                    };
                    if range.length == 0 {
                        return text
                            .carets
                            .iter()
                            .find(|caret| caret.offset as usize == range.location)
                            .and_then(|caret| caret.rect)
                            .map_or(std::ptr::null_mut(), |bounds| {
                                rect(Rect {
                                    origin: Point {
                                        x: entry.root_origin.x + bounds.x,
                                        y: entry.root_origin.y + entry.viewport_height
                                            - bounds.bottom(),
                                    },
                                    size: Size {
                                        width: bounds.width,
                                        height: bounds.height,
                                    },
                                })
                            });
                    }
                    let mut boxes = text.boxes.iter().filter(|item| {
                        (item.start as usize) < end && (item.end as usize) > range.location
                    });
                    let Some(first) = boxes.next() else {
                        return std::ptr::null_mut();
                    };
                    let mut bounds = first.rect;
                    for item in boxes {
                        let right = bounds.right().max(item.rect.right());
                        let bottom = bounds.bottom().max(item.rect.bottom());
                        bounds.x = bounds.x.min(item.rect.x);
                        bounds.y = bounds.y.min(item.rect.y);
                        bounds.width = right - bounds.x;
                        bounds.height = bottom - bounds.y;
                    }
                    rect(Rect {
                        origin: Point {
                            x: entry.root_origin.x + bounds.x,
                            y: entry.root_origin.y + entry.viewport_height - bounds.bottom(),
                        },
                        size: Size {
                            width: bounds.width,
                            height: bounds.height,
                        },
                    })
                }
                _ => std::ptr::null_mut(),
            }
        }
    })
}

unsafe fn register() -> Result<Id, String> {
    let existing = unsafe { class("OrynAccessibilityElement") };
    if !existing.is_null() {
        return Ok(existing);
    }
    let name = CString::new("OrynAccessibilityElement").unwrap();
    let class =
        unsafe { objc_allocateClassPair(self::class("NSAccessibilityElement"), name.as_ptr(), 0) };
    if class.is_null() {
        return Err("cannot allocate accessibility class".into());
    }
    for (name, imp, encoding) in [
        (
            "accessibilityParameterizedAttributeNames",
            parameter_names as *const c_void,
            "@@:",
        ),
        (
            "accessibilityAttributeValue:forParameter:",
            parameter as *const c_void,
            "@@:@@",
        ),
        ("accessibilityIsIgnored", ignored as *const c_void, "c@:"),
        ("isAccessibilityElement", element as *const c_void, "c@:"),
        (
            "accessibilityAttributeNames",
            attribute_names as *const c_void,
            "@@:",
        ),
        (
            "accessibilityAttributeValue:",
            attribute as *const c_void,
            "@@:@",
        ),
        (
            "accessibilityIsAttributeSettable:",
            settable as *const c_void,
            "c@:@",
        ),
        (
            "accessibilitySetValue:forAttribute:",
            set_attribute as *const c_void,
            "v@:@@",
        ),
        ("accessibilityActionNames", actions as *const c_void, "@@:"),
        (
            "accessibilityPerformAction:",
            perform as *const c_void,
            "v@:@",
        ),
        ("accessibilityPerformPress", press as *const c_void, "c@:"),
        (
            "accessibilityPerformIncrement",
            increment as *const c_void,
            "c@:",
        ),
        (
            "accessibilityPerformDecrement",
            decrement as *const c_void,
            "c@:",
        ),
        (
            "accessibilityFocusedUIElement",
            focused as *const c_void,
            "@@:",
        ),
        (
            "accessibilityHitTest:",
            hit_test as *const c_void,
            "@@:{CGPoint=dd}",
        ),
    ] {
        let encoding = if cfg!(target_arch = "aarch64") && encoding.starts_with('c') {
            format!("B{}", &encoding[1..])
        } else {
            encoding.to_owned()
        };
        let encoding = CString::new(encoding).unwrap();
        if !unsafe { class_addMethod(class, sel(name), imp, encoding.as_ptr()) } {
            return Err(format!("cannot install {name}"));
        }
    }
    unsafe { objc_registerClassPair(class) };
    Ok(class)
}
/// Main-thread-only actual NSWindow/NSView host; !Send and !Sync by construction.
pub struct Host {
    pip: picture_in_picture::Host,
    pending_notifications: Vec<(SemanticRef, &'static str)>,
    columns: BTreeMap<columns::Key, Id>,
    marker_namespace: [u8; 16],
    application: Id,
    window: Id,
    view: Id,
    image_view: Id,
    class: Id,
    objects: BTreeMap<SemanticRef, Id>,
    sender: NativeActionSender,
    announced: live::Tracker,
    input_scope: Option<(
        crate::host::webdriver_script::AutomationRealmSnapshot,
        SemanticRef,
    )>,
    input_keys: BTreeMap<u16, Vec<u16>>,
    input_modifiers: u64,
    input_height: f64,
    input_width: f64,
    input_dirty: bool,
    input_active: bool,
    input_wheel: (f64, f64),
    failure: Option<String>,
    marker: PhantomData<Rc<()>>,
}
impl Host {
    pub fn new(sender: SyncSender<ActionRequest>) -> Result<Self, String> {
        Self::new_inner(NativeActionSender::Plain(sender))
    }
    #[doc(hidden)]
    pub fn new_owned(sender: SyncSender<OwnedActionRequest>) -> Result<Self, String> {
        Self::new_inner(NativeActionSender::Owned(sender))
    }
    fn new_inner(sender: NativeActionSender) -> Result<Self, String> {
        unsafe {
            let mut marker_namespace = [0; 16];
            getrandom::fill(&mut marker_namespace)
                .map_err(|error| format!("native AX marker namespace: {error}"))?;
            if pthread_main_np() != 1 {
                return Err("accessibility host requires the AppKit main thread".into());
            }
            let pool = object(class("NSAutoreleasePool"), "new");
            let application = object(class("NSApplication"), "sharedApplication");
            let policy: unsafe extern "C" fn(Id, Sel, isize) -> i8 =
                std::mem::transmute(objc_msgSend as *const ());
            policy(application, sel("setActivationPolicy:"), 0);
            let init: unsafe extern "C" fn(Id, Sel, Rect, usize, usize, i8) -> Id =
                std::mem::transmute(objc_msgSend as *const ());
            let window = init(
                object(class("NSWindow"), "alloc"),
                sel("initWithContentRect:styleMask:backing:defer:"),
                Rect {
                    origin: Point { x: 100., y: 100. },
                    size: Size {
                        width: 1280.,
                        height: 720.,
                    },
                },
                1 | 2 | 4,
                2,
                0,
            );
            if window.is_null() {
                void(pool, "drain");
                return Err("cannot create accessibility window".into());
            }
            let boolean: unsafe extern "C" fn(Id, Sel, i8) =
                std::mem::transmute(objc_msgSend as *const ());
            boolean(window, sel("setReleasedWhenClosed:"), 0);
            boolean(window, sel("setAcceptsMouseMovedEvents:"), 1);
            let init_view: unsafe extern "C" fn(Id, Sel, Rect) -> Id =
                std::mem::transmute(objc_msgSend as *const ());
            let view = init_view(
                object(text_input::register()?, "alloc"),
                sel("initWithFrame:"),
                Rect {
                    origin: Point { x: 0., y: 0. },
                    size: Size {
                        width: 1280.,
                        height: 720.,
                    },
                },
            );
            if view.is_null() {
                return Err("native text input view allocation failed".into());
            }
            void_arg(window, "setContentView:", view);
            void(view, "release");
            text_input::install(view, sender.clone());
            let responder: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
                std::mem::transmute(objc_msgSend as *const ());
            responder(window, sel("makeFirstResponder:"), view);
            let image_view = init_view(
                object(class("NSImageView"), "alloc"),
                sel("initWithFrame:"),
                Rect {
                    origin: Point { x: 0., y: 0. },
                    size: Size {
                        width: 1280.,
                        height: 720.,
                    },
                },
            );
            boolean(image_view, sel("setAccessibilityElement:"), 0);
            void_arg(view, "addSubview:", image_view);
            boolean(view, sel("setAccessibilityElement:"), 0);
            void_arg(window, "setTitle:", string("Oryn native accessibility"));
            void_arg(window, "makeKeyAndOrderFront:", std::ptr::null_mut());
            boolean(application, sel("activateIgnoringOtherApps:"), 1);
            void(application, "finishLaunching");
            let class = register()?;
            void(pool, "drain");
            Ok(Self {
                pip: Default::default(),
                pending_notifications: Vec::new(),
                columns: BTreeMap::new(),
                marker_namespace,
                application,
                window,
                view,
                image_view,
                class,
                objects: BTreeMap::new(),
                sender,
                announced: live::Tracker::default(),
                input_scope: None,
                input_keys: BTreeMap::new(),
                input_modifiers: 0,
                input_height: 720.,
                input_width: 1280.,
                input_dirty: false,
                input_active: true,
                input_wheel: (0., 0.),
                failure: None,
                marker: PhantomData,
            })
        }
    }
    pub fn publish(&mut self, snapshot: Snapshot) -> Result<(), String> {
        let column_keys = columns::prepare(&snapshot)?;
        unsafe {
            if pthread_main_np() != 1 {
                return Err("accessibility publication requires main thread".into());
            }
            let pool = object(class("NSAutoreleasePool"), "new");
            let next_scope = snapshot.root_scope.clone().map(|scope| {
                let target = SemanticRef {
                    page_id: snapshot.page,
                    document_generation: snapshot.generation,
                    node: (u64::from(scope.document_root.generation) << 32)
                        | u64::from(scope.document_root.slot),
                };
                (scope, target)
            });
            if self.input_scope != next_scope {
                self.clear_os_input();
            }
            self.input_height = snapshot.viewport.height;
            self.input_width = snapshot.viewport.width;
            self.input_scope = next_scope;
            text_input::publish(self.view, &snapshot, self.input_scope.clone());
            let resize: unsafe extern "C" fn(Id, Sel, Size) =
                std::mem::transmute(objc_msgSend as *const ());
            resize(
                self.window,
                sel("setContentSize:"),
                Size {
                    width: snapshot.viewport.width,
                    height: snapshot.viewport.height,
                },
            );
            let set_frame: unsafe extern "C" fn(Id, Sel, Rect) =
                std::mem::transmute(objc_msgSend as *const ());
            set_frame(
                self.image_view,
                sel("setFrame:"),
                Rect {
                    origin: Point { x: 0., y: 0. },
                    size: Size {
                        width: snapshot.viewport.width,
                        height: snapshot.viewport.height,
                    },
                },
            );
            if let Some(bytes) = &snapshot.image {
                let data: unsafe extern "C" fn(Id, Sel, *const u8, usize) -> Id =
                    std::mem::transmute(objc_msgSend as *const ());
                let data = data(
                    class("NSData"),
                    sel("dataWithBytes:length:"),
                    bytes.as_ptr(),
                    bytes.len(),
                );
                let image = object_arg(object(class("NSImage"), "alloc"), "initWithData:", data);
                if image.is_null() {
                    void(pool, "drain");
                    return Err("native accessibility screenshot decoding failed".into());
                }
                void_arg(self.image_view, "setImage:", image);
                void(image, "release");
            }
            let removed: Vec<_> = self
                .objects
                .keys()
                .filter(|id| !snapshot.nodes.contains_key(id))
                .copied()
                .collect();
            let mut retired = Vec::new();
            for id in removed {
                if let Some(object) = self.objects.remove(&id) {
                    ENTRIES.with(|e| {
                        e.borrow_mut().remove(&(object as usize));
                    });
                    retired.push(object);
                }
            }
            for id in snapshot.nodes.keys() {
                if !self.objects.contains_key(id) {
                    let object = object(object(self.class, "alloc"), "init");
                    if object.is_null() {
                        return Err("accessibility element allocation failed".into());
                    }
                    self.objects.insert(*id, object);
                }
            }
            // AppKit conversion incorporates actual window origin, screen and backing scale.
            let convert_view: unsafe extern "C" fn(Id, Sel, Point, Id) -> Point =
                std::mem::transmute(objc_msgSend as *const ());
            let convert_screen: unsafe extern "C" fn(Id, Sel, Point) -> Point =
                std::mem::transmute(objc_msgSend as *const ());
            let table_objects = Rc::new(self.objects.clone());
            let tables = snapshot
                .tables
                .values()
                .map(|table| Rc::new(table.clone()))
                .collect::<Vec<_>>();
            for (id, node) in &snapshot.nodes {
                let object = self.objects[id];
                let screen = node.rect.map(|rect| {
                    let local = Point {
                        x: rect.x,
                        y: snapshot.viewport.height - rect.y - rect.height,
                    };
                    let window = convert_view(
                        self.view,
                        sel("convertPoint:toView:"),
                        local,
                        std::ptr::null_mut(),
                    );
                    let screen = convert_screen(self.window, sel("convertPointToScreen:"), window);
                    Rect {
                        origin: screen,
                        size: Size {
                            width: rect.width,
                            height: rect.height,
                        },
                    }
                });
                let entry = Entry {
                    marker_namespace: self.marker_namespace,
                    marker_scope: node.scope.clone().or_else(|| snapshot.root_scope.clone()),
                    table: tables
                        .iter()
                        .find(|table| {
                            table.root == *id
                                || table.rows.contains(id)
                                || table.cells.contains_key(id)
                        })
                        .cloned(),
                    table_objects: table_objects.clone(),
                    node: node.clone(),
                    revision: node.revision,
                    parent: node
                        .parent
                        .and_then(|p| self.objects.get(&p).copied())
                        .unwrap_or(self.view),
                    children: node
                        .children
                        .iter()
                        .filter_map(|id| self.objects.get(id).copied())
                        .collect(),
                    relations: node
                        .relations
                        .iter()
                        .map(|(relation, ids)| {
                            (
                                relation.clone(),
                                ids.iter()
                                    .filter_map(|id| self.objects.get(id).copied())
                                    .collect(),
                            )
                        })
                        .collect(),
                    window: self.window,
                    screen,
                    root_origin: convert_screen(
                        self.window,
                        sel("convertPointToScreen:"),
                        convert_view(
                            self.view,
                            sel("convertPoint:toView:"),
                            Point { x: 0., y: 0. },
                            std::ptr::null_mut(),
                        ),
                    ),
                    viewport_height: snapshot.viewport.height,
                    sender: self.sender.clone(),
                };
                let previous =
                    ENTRIES.with(|entries| entries.borrow_mut().insert(object as usize, entry));
                if let Some(previous) = previous {
                    for notification in
                        super::role_mapping::changed_notifications(&previous.node, node)
                    {
                        let target = if notification == "AXSelectedChildrenChanged" {
                            node.parent.unwrap_or(*id)
                        } else {
                            *id
                        };
                        if !self.pending_notifications.contains(&(target, notification)) {
                            self.pending_notifications.push((target, notification));
                        }
                    }
                    let focused = node.states.contains(&NodeState::Focused);
                    let was_focused = previous.node.states.contains(&NodeState::Focused);
                    let active = previous
                        .node
                        .relations
                        .get(&AccessibilityRelation::ActiveDescendant)
                        != node.relations.get(&AccessibilityRelation::ActiveDescendant);
                    if focused && (!was_focused || active) {
                        let target = node
                            .relations
                            .get(&AccessibilityRelation::ActiveDescendant)
                            .and_then(|targets| targets.first())
                            .copied()
                            .unwrap_or(*id);
                        if !self
                            .pending_notifications
                            .contains(&(target, "AXFocusedUIElementChanged"))
                        {
                            self.pending_notifications
                                .push((target, "AXFocusedUIElementChanged"));
                        }
                    }
                }
            }
            columns::publish(&mut self.columns, column_keys, &self.objects, &snapshot)?;
            // Publish all native entries before synchronous AppKit clients read any
            // notification. Retain changes across a fallible column publication retry.
            for (target, notification) in std::mem::take(&mut self.pending_notifications) {
                if let Some(object) = self.objects.get(&target) {
                    NSAccessibilityPostNotification(*object, string(notification));
                }
            }

            for live in self.announced.update(&snapshot.live_regions)? {
                let values = [
                    string(&live.text),
                    integer(if live.assertive { 90 } else { 50 }),
                ];
                let keys = [NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey];
                let dictionary: unsafe extern "C" fn(Id, Sel, *const Id, *const Id, usize) -> Id =
                    std::mem::transmute(objc_msgSend as *const ());
                let info = dictionary(
                    class("NSDictionary"),
                    sel("dictionaryWithObjects:forKeys:count:"),
                    values.as_ptr(),
                    keys.as_ptr(),
                    2,
                );
                NSAccessibilityPostNotificationWithUserInfo(
                    self.window,
                    NSAccessibilityAnnouncementRequestedNotification,
                    info,
                );
            }
            let roots: Vec<_> = snapshot
                .roots
                .iter()
                .filter_map(|id| self.objects.get(id).copied())
                .collect();
            void_arg(self.view, "setAccessibilityChildren:", array(&roots));
            for object in retired {
                NSAccessibilityPostNotification(object, string("AXUIElementDestroyed"));
                void(object, "release");
            }
            void_arg(self.window, "setTitle:", string(&snapshot.title));
            NSAccessibilityPostNotification(self.view, string("AXLayoutChanged"));
            let file_result = file_picker::publish(self.window, &self.sender, &snapshot);
            void(pool, "drain");
            file_result
        }
    }
    /// One bounded native run-loop iteration; callers publish updates between turns.
    pub fn pump(&mut self) -> bool {
        unsafe {
            if let Err(error) = self.pip.pump() {
                self.failure = Some(error);
            }
            if let Some(error) = text_input::failure(self.view) {
                self.failure = Some(error);
            }
            let active: unsafe extern "C" fn(Id, Sel) -> i8 =
                std::mem::transmute(objc_msgSend as *const ());
            let active = active(self.application, sel("isActive")) != 0
                && active(self.window, sel("isKeyWindow")) != 0;
            if self.input_active
                && !active
                && let Err(error) = self.cancel_os_input()
            {
                self.failure = Some(error);
            }
            self.input_active = active;
            text_input::update_secure_input(self.view, active);
            let pool = object(class("NSAutoreleasePool"), "new");
            let deadline: unsafe extern "C" fn(Id, Sel, f64) -> Id =
                std::mem::transmute(objc_msgSend as *const ());
            let date = deadline(class("NSDate"), sel("dateWithTimeIntervalSinceNow:"), 0.02);
            let next: unsafe extern "C" fn(Id, Sel, usize, Id, Id, i8) -> Id =
                std::mem::transmute(objc_msgSend as *const ());
            let deferred = if active {
                text_input::next_event(self.view)
            } else {
                None
            };
            let event = if let Some(event) = &deferred {
                event.event
            } else {
                next(
                    self.application,
                    sel("nextEventMatchingMask:untilDate:inMode:dequeue:"),
                    usize::MAX,
                    date,
                    string("kCFRunLoopDefaultMode"),
                    1,
                )
            };
            if !event.is_null() {
                match self.dispatch_os_event(event) {
                    Ok(true) => {}
                    Ok(false) => void_arg(self.application, "sendEvent:", event),
                    Err(error) => self.failure = Some(error),
                }
            }
            void(self.application, "updateWindows");
            let visible: unsafe extern "C" fn(Id, Sel) -> i8 =
                std::mem::transmute(objc_msgSend as *const ());
            let result = visible(self.window, sel("isVisible")) != 0;
            void(pool, "drain");
            result && self.failure.is_none()
        }
    }
    pub fn run(mut self, updates: Receiver<Result<Update, String>>) -> Result<(), String> {
        while self.pump() {
            match updates.try_recv() {
                Ok(Ok(Update::Snapshot(snapshot))) => self.publish(*snapshot)?,
                Ok(Ok(Update::Receipts(receipts))) => text_input::receipts(self.view, receipts)?,
                Ok(Ok(Update::OwnedReceipts(receipts))) => text_input::receipts_owned(self.view,receipts)?,
                Ok(Err(error)) => return Err(error),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(error) = self.failure.take() {
            Err(error)
        } else {
            Ok(())
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            let pool = object(class("NSAutoreleasePool"), "new");
            text_input::remove(self.view);
            file_picker::retire(self.window);
            columns::retire(&mut self.columns);
            void_arg(self.view, "setAccessibilityChildren:", array(&[]));
            for (_, object) in std::mem::take(&mut self.objects) {
                ENTRIES.with(|entries| {
                    entries.borrow_mut().remove(&(object as usize));
                });
                NSAccessibilityPostNotification(object, string("AXUIElementDestroyed"));
                void(object, "release");
            }
            void(self.image_view, "release");
            void(self.window, "close");
            void(self.window, "release");
            void(pool, "drain");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn main_thread_ownership_is_enforced_before_appkit_access() {
        let result = std::thread::spawn(|| {
            let (sender, _) = std::sync::mpsc::sync_channel(1);
            Host::new(sender).is_err()
        })
        .join()
        .unwrap();
        assert!(result);
    }
    #[test]
    fn retired_native_elements_cannot_enqueue_actions() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<ActionRequest>(1);
        drop(tx);
        assert!(!enqueue(0x1234usize as Id, Action::Press));
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn platform_roles_preserve_controls_and_landmark_containers() {
        assert_eq!(role("checkbox"), "AXCheckBox");
        assert_eq!(role("textbox"), "AXTextField");
        assert_eq!(role("table"), "AXTable");
        assert_eq!(role("navigation"), "AXGroup");
    }
}
