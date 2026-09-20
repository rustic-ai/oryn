//! Main-thread NSTextInputClient. No callback waits for a page worker. A retained
//! NSEvent is interpreted only after its native keydown continuation is accepted.
use super::*;
use crate::webdriver_actions::{Action as Input, ActionsRequest, OsKeyReceipt, Source, SourceKind};
use oryn_js::composition::{Operation, StateSnapshot, TextRange};
pub(super) struct Pending {
    pub(super) event: Id,
    root: Option<(
        crate::host::webdriver_script::AutomationRealmSnapshot,
        SemanticRef,
    )>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        unsafe { void(self.event, "release") }
    }
}
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn EnableSecureEventInput() -> i32;
    fn DisableSecureEventInput() -> i32;
}
#[derive(Default)]
struct SecureInput {
    enabled: bool,
}
impl SecureInput {
    fn transition(
        &mut self,
        wanted: bool,
        mut call: impl FnMut(bool) -> i32,
    ) -> Result<(), String> {
        if self.enabled == wanted {
            return Ok(());
        }
        let status = call(wanted);
        if status != 0 {
            return Err(format!("native secure input transition failed: {status}"));
        }
        self.enabled = wanted;
        Ok(())
    }
    fn set(&mut self, wanted: bool) -> Result<(), String> {
        self.transition(wanted, |enabled| unsafe {
            if enabled {
                EnableSecureEventInput()
            } else {
                DisableSecureEventInput()
            }
        })
    }
}
impl Drop for SecureInput {
    fn drop(&mut self) {
        let _ = self.set(false);
    }
}
struct Client {
    geometry: Option<super::super::text_geometry::TextGeometry>,
    secure: SecureInput,
    sender: SyncSender<ActionRequest>,
    root: Option<(
        crate::host::webdriver_script::AutomationRealmSnapshot,
        SemanticRef,
    )>,
    next: u64,
    pending: BTreeMap<u64, Pending>,
    active: Option<OsKeyReceipt>,
    latest: u64,
    sequence: u64,
    interpreting: bool,
    busy: bool,
    replaying: bool,
    deferred: std::collections::VecDeque<Pending>,
    state: StateSnapshot,
    failure: Option<String>,
}
thread_local! {static CLIENTS:RefCell<BTreeMap<usize,Client>>=const { RefCell::new(BTreeMap::new()) };}
fn empty() -> StateSnapshot {
    StateSnapshot {
        mutation_revision: 0,
        aborted: false,
        protected: false,
        text: Vec::new(),
        selection: TextRange {
            start: 0,
            length: 0,
        },
        marked: None,
    }
}
pub(super) fn install(view: Id, sender: SyncSender<ActionRequest>) {
    CLIENTS.with(|clients| {
        clients.borrow_mut().insert(
            view as usize,
            Client {
                geometry: None,
                secure: SecureInput::default(),
                sender,
                root: None,
                next: 1,
                pending: BTreeMap::new(),
                active: None,
                latest: 0,
                sequence: 1,
                interpreting: false,
                busy: false,
                replaying: false,
                deferred: std::collections::VecDeque::new(),
                state: empty(),
                failure: None,
            },
        );
    });
}
pub(super) fn remove(view: Id) {
    CLIENTS.with(|clients| {
        clients.borrow_mut().remove(&(view as usize));
    });
}
pub(super) fn clear(view: Id) {
    CLIENTS.with(|clients| {
        if let Some(client) = clients.borrow_mut().get_mut(&(view as usize)) {
            if let Err(error) = client.secure.set(false) {
                client.failure = Some(error);
            }
            client.pending.clear();
            client.deferred.clear();
            client.busy = false;
            client.replaying = false;
            client.active = None;
            client.interpreting = false;
            client.latest = 0;
            client.state = empty();
            client.geometry = None;
        }
    });
}
pub(super) fn update_secure_input(view: Id, active: bool) {
    CLIENTS.with(|clients| {
        if let Some(client) = clients.borrow_mut().get_mut(&(view as usize))
            && let Err(error) = client.secure.set(active && client.state.protected)
        {
            client.failure = Some(error);
        }
    });
}
pub(super) fn failure(view: Id) -> Option<String> {
    CLIENTS.with(|clients| {
        clients
            .borrow_mut()
            .get_mut(&(view as usize))
            .and_then(|client| client.failure.take())
    })
}
pub(super) fn composing(view: Id) -> bool {
    CLIENTS.with(|clients| {
        clients
            .borrow()
            .get(&(view as usize))
            .is_some_and(|client| client.state.marked.is_some())
    })
}
// Serialize native input behind accepted text interpretation. AppKit stays
// responsive; only retained input events wait for the worker continuation.
pub(super) fn defer_event(view: Id, event: Id) -> Result<bool, String> {
    CLIENTS.with(|clients| {
        let mut clients = clients.borrow_mut();
        let client = clients
            .get_mut(&(view as usize))
            .ok_or("native text view retired")?;
        if client.replaying {
            client.replaying = false;
            return Ok(false);
        }
        if !client.busy && client.deferred.is_empty() {
            return Ok(false);
        }
        if client.deferred.len() >= 512 {
            return Err("native input continuation backlog exceeded".into());
        }
        unsafe {
            object(event, "retain");
        }
        client.deferred.push_back(Pending {
            event,
            root: client.root.clone(),
        });
        Ok(true)
    })
}
pub(super) fn next_event(view: Id) -> Option<Pending> {
    CLIENTS.with(|clients| {
        let mut clients = clients.borrow_mut();
        let client = clients.get_mut(&(view as usize))?;
        if client.busy {
            return None;
        }
        let event = client.deferred.pop_front()?;
        if event.root != client.root {
            return None;
        }
        client.replaying = true;
        Some(event)
    })
}
pub(super) fn reserve(view: Id, event: Id) -> Result<(u64, bool), String> {
    CLIENTS.with(|clients| {
        let mut clients = clients.borrow_mut();
        let client = clients
            .get_mut(&(view as usize))
            .ok_or("native text client retired")?;
        if client.pending.len() >= 128 {
            return Err("native key interpretation backlog exceeded".into());
        }
        client.busy = true;
        let token = client.next;
        client.next = token.checked_add(1).ok_or("native input token exhausted")?;
        unsafe {
            object(event, "retain");
        }
        client.pending.insert(
            token,
            Pending {
                event,
                root: client.root.clone(),
            },
        );
        Ok((token, client.state.marked.is_some()))
    })
}
pub(super) fn publish(
    view: Id,
    snapshot: &Snapshot,
    root: Option<(
        crate::host::webdriver_script::AutomationRealmSnapshot,
        SemanticRef,
    )>,
) {
    CLIENTS.with(|clients| {
        let mut clients = clients.borrow_mut();
        let Some(client) = clients.get_mut(&(view as usize)) else {
            return;
        };
        if client.root != root {
            if let Err(error) = client.secure.set(false) {
                client.failure = Some(error);
            }
            client.pending.clear();
            client.deferred.clear();
            client.busy = false;
            client.replaying = false;
            client.active = None;
            client.state = empty();
        }
        client.root = root;
        client.geometry = None;
        let node = if let Some(active) = &client.active {
            snapshot.nodes.values().find(|node| {
                active.target.is_some_and(|target| {
                    node.id.node == ((u64::from(target.generation) << 32) | u64::from(target.slot))
                }) && node
                    .scope
                    .as_ref()
                    .or(snapshot.root_scope.as_ref())
                    .is_some_and(|scope| {
                        scope.realm == active.context.realm
                            && scope.document_generation == active.context.document_generation
                            && scope.stream_epoch == active.context.stream_epoch
                    })
            })
        } else {
            snapshot
                .nodes
                .values()
                .find(|node| node.states.contains(&NodeState::Focused) && node.text.is_some())
        };
        if let Some(node) = node
            && let Some(text) = &node.text
        {
            if client.active.is_none() {
                client.state = StateSnapshot {
                    mutation_revision: text
                        .protected_caret
                        .as_ref()
                        .map_or(0, |record| record.mutation_revision),
                    aborted: false,
                    protected: text.protected,
                    text: text.value.clone(),
                    selection: text.protected_caret.as_ref().map_or(
                        TextRange {
                            start: text.start,
                            length: text.end.saturating_sub(text.start),
                        },
                        |record| record.selection,
                    ),
                    marked: None,
                };
            }
            if let Some(record) = text
                .protected_caret
                .as_ref()
                .filter(|record| record.matches(&client.state))
            {
                let mut caret = record.caret.clone();
                caret.rect = caret
                    .rect
                    .map(|rect| unsafe { screen_rect(view, snapshot.viewport.height, rect) });
                client.geometry = Some(super::super::text_geometry::TextGeometry {
                    boxes: Vec::new(),
                    carets: vec![caret],
                });
            }
            if text.protected == client.state.protected
                && text.value == client.state.text
                && !text.protected
            {
                let mut geometry = super::super::text_geometry::TextGeometry {
                    boxes: text.boxes.clone(),
                    carets: text.carets.clone(),
                };
                for item in &mut geometry.boxes {
                    item.rect = unsafe { screen_rect(view, snapshot.viewport.height, item.rect) };
                }
                for caret in &mut geometry.carets {
                    caret.rect = caret
                        .rect
                        .map(|rect| unsafe { screen_rect(view, snapshot.viewport.height, rect) });
                }
                client.geometry = Some(geometry);
            }
        }
    });
    unsafe {
        let context = object(view, "inputContext");
        if !context.is_null() {
            void(context, "invalidateCharacterCoordinates");
        }
    }
}
unsafe fn screen_rect(
    view: Id,
    height: f64,
    rect: oryn_common::v4::CssRect,
) -> oryn_common::v4::CssRect {
    unsafe {
        let convert_view: unsafe extern "C" fn(Id, Sel, Point, Id) -> Point =
            std::mem::transmute(objc_msgSend as *const ());
        let convert_screen: unsafe extern "C" fn(Id, Sel, Point) -> Point =
            std::mem::transmute(objc_msgSend as *const ());
        let local = convert_view(
            view,
            sel("convertPoint:toView:"),
            Point {
                x: rect.x,
                y: height - rect.bottom(),
            },
            std::ptr::null_mut(),
        );
        let origin = convert_screen(object(view, "window"), sel("convertPointToScreen:"), local);
        oryn_common::v4::CssRect {
            x: origin.x,
            y: origin.y,
            width: rect.width,
            height: rect.height,
        }
    }
}
fn queue(client: &mut Client, operation: Operation) -> Result<(), String> {
    let grant = client
        .active
        .as_ref()
        .ok_or("no native text interpretation grant")?;
    let (root, target) = client.root.as_ref().ok_or("native text root retired")?;
    let sequence = client.sequence;
    client.sequence = sequence
        .checked_add(1)
        .ok_or("native text sequence exhausted")?;
    let request = ActionsRequest {
        sources: vec![Source {
            id: grant.source.clone(),
            kind: SourceKind::Key,
            actions: vec![Input::OsText {
                token: grant.token,
                sequence,
                operation,
            }],
        }],
    };
    client
        .sender
        .try_send(ActionRequest {
            scope: Some(root.clone()),
            target: *target,
            revision: Revision(0),
            action: Action::Input(request),
        })
        .map_err(|_| "native text input queue unavailable".into())
}
fn change(view: Id, operation: Operation) {
    CLIENTS.with(|clients| {
        let Ok(mut clients) = clients.try_borrow_mut() else {
            return;
        };
        let Some(client) = clients.get_mut(&(view as usize)) else {
            return;
        };
        if client.active.is_none() && matches!(operation, Operation::Unmark | Operation::Cancel) {
            return;
        }
        if !operation.validate() {
            client.failure = Some("invalid native text operation".into());
            return;
        }
        if let Err(error) = queue(client, operation.clone()) {
            client.failure = Some(error);
            return;
        }
        match operation {
            Operation::SetMarked {
                text,
                selected,
                replacement,
            } => {
                let replace = replacement
                    .or(client.state.marked)
                    .unwrap_or(client.state.selection);
                if !client.state.protected {
                    if let Some(end) = replace
                        .end()
                        .filter(|end| *end as usize <= client.state.text.len())
                    {
                        client
                            .state
                            .text
                            .splice(replace.start as usize..end as usize, text.iter().copied());
                    } else {
                        client.failure = Some("native marked range unavailable".into());
                        return;
                    }
                }
                client.state.marked = Some(TextRange {
                    start: replace.start,
                    length: text.len() as u32,
                });
                client.state.selection = TextRange {
                    start: replace.start.saturating_add(selected.start),
                    length: selected.length,
                };
            }
            Operation::Commit { text, replacement } => {
                let replace = replacement
                    .or(client.state.marked)
                    .unwrap_or(client.state.selection);
                if !client.state.protected
                    && let Some(end) = replace
                        .end()
                        .filter(|end| *end as usize <= client.state.text.len())
                {
                    client
                        .state
                        .text
                        .splice(replace.start as usize..end as usize, text.iter().copied());
                }
                client.state.selection = TextRange {
                    start: replace.start.saturating_add(text.len() as u32),
                    length: 0,
                };
                client.state.marked = None;
            }
            Operation::Command(_) => {
                client.state.marked = None;
            }
            Operation::Unmark => {
                client.state.marked = None;
                if !client.interpreting {
                    if let Err(error) = queue(client, Operation::Finish { keep_marked: false }) {
                        client.failure = Some(error);
                    }
                    client.active = None;
                }
            }
            Operation::Cancel => {
                client.state.marked = None;
                client.active = None;
            }
            _ => {}
        }
        client.geometry = None;
    });
}
pub(super) fn receipts(view: Id, receipts: Vec<OsKeyReceipt>) -> Result<(), String> {
    for receipt in receipts {
        let aborted = receipt.state.as_ref().is_some_and(|state| state.aborted);
        let pending = CLIENTS.with(|clients| {
            let mut clients = clients.borrow_mut();
            let client = clients
                .get_mut(&(view as usize))
                .ok_or("native text view retired")?;
            if receipt.continuation_done && receipt.token >= client.latest {
                client.busy = false;
            }
            let pending = client.pending.remove(&receipt.token);
            if let Some(pending) = &pending {
                if pending.root != client.root {
                    return Ok(None);
                }
                if aborted {
                    client.latest = receipt.token;
                    client.active = None;
                    if let Some(state) = &receipt.state {
                        client.state = state.clone();
                    }
                    return Ok(None);
                }
                if !receipt.accepted || !receipt.interpret {
                    return Ok(None);
                }
                client.latest = receipt.token;
                client.sequence = 1;
                client.interpreting = true;
                client.active = Some(receipt.clone());
            }
            if client.latest == receipt.token
                && let Some(state) = &receipt.state
            {
                client.state = state.clone();
                client.geometry = None;
                if state.aborted {
                    client.active = None;
                }
            }
            Ok::<_, String>(pending)
        })?;
        if aborted {
            unsafe {
                let context = object(view, "inputContext");
                if !context.is_null() {
                    void(context, "discardMarkedText");
                }
            }
        }
        if let Some(pending) = pending {
            // No RefCell borrow crosses AppKit: interpretKeyEvents synchronously calls
            // our text-client methods, which enqueue their ordered native operations.
            unsafe {
                void_arg(view, "interpretKeyEvents:", array(&[pending.event]));
            }
            CLIENTS.with(|clients| {
                let mut clients = clients.borrow_mut();
                if let Some(client) = clients.get_mut(&(view as usize)) {
                    client.interpreting = false;
                    if client
                        .active
                        .as_ref()
                        .is_some_and(|active| active.token == receipt.token)
                    {
                        let keep_marked = client.state.marked.is_some();
                        if let Err(error) = queue(client, Operation::Finish { keep_marked }) {
                            client.failure = Some(error);
                        }
                        if !keep_marked {
                            client.active = None;
                        }
                    }
                }
            });
        }
    }
    Ok(())
}
fn state<T>(view: Id, default: T, f: impl FnOnce(&Client) -> T) -> T {
    CLIENTS.with(|clients| {
        clients
            .try_borrow()
            .ok()
            .and_then(|clients| clients.get(&(view as usize)).map(f))
            .unwrap_or(default)
    })
}
fn replacement_range(range: Range) -> Result<Option<TextRange>, &'static str> {
    if range.location == usize::MAX {
        return Ok(None);
    }
    text_range(range)
        .map(Some)
        .ok_or("native replacement range overflow")
}
fn invalid(view: Id, message: &str) {
    CLIENTS.with(|clients| {
        if let Some(client) = clients.borrow_mut().get_mut(&(view as usize)) {
            client.failure = Some(message.into());
        }
    });
}
fn text_range(range: Range) -> Option<TextRange> {
    if range.location == usize::MAX {
        return None;
    }
    let start = u32::try_from(range.location).ok()?;
    let length = u32::try_from(range.length).ok()?;
    start.checked_add(length)?;
    Some(TextRange { start, length })
}
unsafe fn attributed_units(value: Id) -> Option<Vec<u16>> {
    if let Some(text) = unsafe { native_units(value) } {
        return Some(text);
    }
    if value.is_null() {
        return None;
    }
    let check: unsafe extern "C" fn(Id, Sel, Id) -> i8 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    if unsafe { check(value, sel("isKindOfClass:"), class("NSAttributedString")) } == 0 {
        return None;
    }
    unsafe { native_units(object(value, "string")) }
}
unsafe extern "C" fn accepts(_: Id, _: Sel) -> i8 {
    1
}
unsafe extern "C" fn has_marked(view: Id, _: Sel) -> i8 {
    composing(view) as i8
}
unsafe extern "C" fn marked(view: Id, _: Sel) -> Range {
    state(
        view,
        Range {
            location: usize::MAX,
            length: 0,
        },
        |client| {
            client.state.marked.map_or(
                Range {
                    location: usize::MAX,
                    length: 0,
                },
                |range| Range {
                    location: range.start as usize,
                    length: range.length as usize,
                },
            )
        },
    )
}
unsafe extern "C" fn selected(view: Id, _: Sel) -> Range {
    state(
        view,
        Range {
            location: usize::MAX,
            length: 0,
        },
        |client| Range {
            location: client.state.selection.start as usize,
            length: client.state.selection.length as usize,
        },
    )
}
unsafe extern "C" fn valid(_: Id, _: Sel) -> Id {
    unsafe { array(&[]) }
}
unsafe extern "C" fn unmark(view: Id, _: Sel) {
    change(view, Operation::Unmark);
}
unsafe extern "C" fn insert(view: Id, _: Sel, value: Id, replacement: Range) {
    match (
        unsafe { attributed_units(value) },
        replacement_range(replacement),
    ) {
        (Some(text), Ok(replacement)) => change(view, Operation::Commit { text, replacement }),
        _ => invalid(view, "invalid native committed text or replacement range"),
    }
}
unsafe extern "C" fn mark(view: Id, _: Sel, value: Id, selection: Range, replacement: Range) {
    match (
        unsafe { attributed_units(value) },
        text_range(selection),
        replacement_range(replacement),
    ) {
        (Some(text), Some(selected), Ok(replacement)) => change(
            view,
            Operation::SetMarked {
                text,
                selected,
                replacement,
            },
        ),
        _ => invalid(view, "invalid native marked text or replacement range"),
    }
}
unsafe extern "C" fn substring(view: Id, _: Sel, proposed: Range, actual: *mut Range) -> Id {
    state(view, std::ptr::null_mut(), |client| {
        if client.state.protected {
            return std::ptr::null_mut();
        }
        let Some(range) = text_range(proposed) else {
            return std::ptr::null_mut();
        };
        let Some(end) = range
            .end()
            .filter(|end| *end as usize <= client.state.text.len())
        else {
            return std::ptr::null_mut();
        };
        unsafe {
            if !actual.is_null() {
                *actual = proposed;
            }
            let value = utf16(&client.state.text[range.start as usize..end as usize]);
            object(
                object_arg(
                    object(class("NSAttributedString"), "alloc"),
                    "initWithString:",
                    value,
                ),
                "autorelease",
            )
        }
    })
}
unsafe extern "C" fn first_rect(view: Id, _: Sel, proposed: Range, actual: *mut Range) -> Rect {
    let empty = Rect {
        origin: Point { x: 0., y: 0. },
        size: Size {
            width: 0.,
            height: 0.,
        },
    };
    state(view, empty, |client| {
        let Some((range, rect)) = text_range(proposed).and_then(|range| {
            let geometry = client.geometry.as_ref()?;
            let range = if client.state.protected {
                TextRange {
                    start: geometry.carets.first()?.offset,
                    length: 0,
                }
            } else {
                range
            };
            geometry.first_rect(range)
        }) else {
            unsafe {
                if !actual.is_null() {
                    *actual = Range {
                        location: usize::MAX,
                        length: 0,
                    };
                }
            }
            return empty;
        };
        unsafe {
            if !actual.is_null() {
                *actual = Range {
                    location: range.start as usize,
                    length: range.length as usize,
                };
            }
        }
        Rect {
            origin: Point {
                x: rect.x,
                y: rect.y,
            },
            size: Size {
                width: rect.width,
                height: rect.height,
            },
        }
    })
}
unsafe extern "C" fn character_index(view: Id, _: Sel, point: Point) -> usize {
    state(view, usize::MAX, |client| {
        client
            .geometry
            .as_ref()
            .and_then(|geometry| geometry.index_at(point.x, point.y, client.state.selection.start))
            .map_or(usize::MAX, |offset| offset as usize)
    })
}
unsafe extern "C" fn command(view: Id, _: Sel, command: Sel) {
    unsafe {
        use oryn_js::composition::{
            TextCommand as Command, TextDirection as Direction, TextUnit as Unit,
        };
        if command == sel("cancelOperation:") {
            change(view, Operation::Cancel);
            return;
        }
        for (name, backward) in [("insertTab:", false), ("insertBacktab:", true)] {
            if command == sel(name) {
                change(view, Operation::Command(Command::Tab { backward }));
                return;
            }
        }
        for (name, line_break) in [("insertNewline:", false), ("insertLineBreak:", true)] {
            if command == sel(name) {
                change(view, Operation::Command(Command::Newline { line_break }));
                return;
            }
        }
        for (name, backward, unit) in [
            ("deleteBackward:", true, Unit::Character),
            ("deleteForward:", false, Unit::Character),
            ("deleteWordBackward:", true, Unit::Word),
            ("deleteWordForward:", false, Unit::Word),
            ("deleteToBeginningOfLine:", true, Unit::LineBoundary),
            ("deleteToEndOfLine:", false, Unit::LineBoundary),
            (
                "deleteToBeginningOfParagraph:",
                true,
                Unit::ParagraphBoundary,
            ),
            ("deleteToEndOfParagraph:", false, Unit::ParagraphBoundary),
        ] {
            if command == sel(name) {
                change(view, Operation::Command(Command::Delete { backward, unit }));
                return;
            }
        }
        for (name, direction, unit) in [
            ("moveLeft", Direction::Left, Unit::Character),
            ("moveRight", Direction::Right, Unit::Character),
            ("moveBackward", Direction::Backward, Unit::Character),
            ("moveForward", Direction::Forward, Unit::Character),
            ("moveWordLeft", Direction::Left, Unit::Word),
            ("moveWordRight", Direction::Right, Unit::Word),
            ("moveWordBackward", Direction::Backward, Unit::Word),
            ("moveWordForward", Direction::Forward, Unit::Word),
            ("moveUp", Direction::Backward, Unit::Line),
            ("moveDown", Direction::Forward, Unit::Line),
            (
                "moveToBeginningOfLine",
                Direction::Backward,
                Unit::LineBoundary,
            ),
            ("moveToEndOfLine", Direction::Forward, Unit::LineBoundary),
            (
                "moveToBeginningOfParagraph",
                Direction::Backward,
                Unit::ParagraphBoundary,
            ),
            (
                "moveToEndOfParagraph",
                Direction::Forward,
                Unit::ParagraphBoundary,
            ),
            (
                "moveParagraphBackward",
                Direction::Backward,
                Unit::Paragraph,
            ),
            ("moveParagraphForward", Direction::Forward, Unit::Paragraph),
            (
                "moveToBeginningOfDocument",
                Direction::Backward,
                Unit::DocumentBoundary,
            ),
            (
                "moveToEndOfDocument",
                Direction::Forward,
                Unit::DocumentBoundary,
            ),
        ] {
            for extend in [false, true] {
                let selector = format!("{name}{}:", if extend { "AndModifySelection" } else { "" });
                if command == sel(&selector) {
                    change(
                        view,
                        Operation::Command(Command::Move {
                            direction,
                            unit,
                            extend,
                        }),
                    );
                    return;
                }
            }
        }
    }
}

pub(super) unsafe fn register() -> Result<Id, String> {
    unsafe {
        let name = "OrynNativeTextInputView";
        let existing = class(name);
        if !existing.is_null() {
            return Ok(existing);
        }
        let name = CString::new(name).unwrap();
        let view = objc_allocateClassPair(class("NSView"), name.as_ptr(), 0);
        if view.is_null() {
            return Err("native text view class allocation failed".into());
        }
        let protocol = CString::new("NSTextInputClient").unwrap();
        let protocol = objc_getProtocol(protocol.as_ptr());
        if protocol.is_null() || !class_addProtocol(view, protocol) {
            return Err("NSTextInputClient protocol unavailable".into());
        }
        for (name, method, encoding) in [
            ("acceptsFirstResponder", accepts as *const c_void, "c@:"),
            ("hasMarkedText", has_marked as *const c_void, "c@:"),
            ("markedRange", marked as *const c_void, "{_NSRange=QQ}@:"),
            (
                "selectedRange",
                selected as *const c_void,
                "{_NSRange=QQ}@:",
            ),
            (
                "validAttributesForMarkedText",
                valid as *const c_void,
                "@@:",
            ),
            ("unmarkText", unmark as *const c_void, "v@:"),
            (
                "insertText:replacementRange:",
                insert as *const c_void,
                "v@:@{_NSRange=QQ}",
            ),
            (
                "setMarkedText:selectedRange:replacementRange:",
                mark as *const c_void,
                "v@:@{_NSRange=QQ}{_NSRange=QQ}",
            ),
            (
                "attributedSubstringForProposedRange:actualRange:",
                substring as *const c_void,
                "@@:{_NSRange=QQ}^{_NSRange=QQ}",
            ),
            (
                "firstRectForCharacterRange:actualRange:",
                first_rect as *const c_void,
                "{CGRect={CGPoint=dd}{CGSize=dd}}@:{_NSRange=QQ}^{_NSRange=QQ}",
            ),
            (
                "characterIndexForPoint:",
                character_index as *const c_void,
                "Q@:{CGPoint=dd}",
            ),
            ("doCommandBySelector:", command as *const c_void, "v@::"),
        ] {
            let encoding = if cfg!(target_arch = "aarch64") && encoding.starts_with('c') {
                format!("B{}", &encoding[1..])
            } else {
                encoding.into()
            };
            let encoding = CString::new(encoding).unwrap();
            if !class_addMethod(view, sel(name), method, encoding.as_ptr()) {
                return Err(format!("cannot install native text method {name}"));
            }
        }
        objc_registerClassPair(view);
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secure_input_balances_transitions_and_retries_failed_release() {
        let mut secure = SecureInput::default();
        let mut calls = Vec::new();
        secure
            .transition(true, |enabled| {
                calls.push(enabled);
                0
            })
            .unwrap();
        secure
            .transition(true, |enabled| {
                calls.push(enabled);
                0
            })
            .unwrap();
        assert!(secure.transition(false, |_| -1).is_err());
        assert!(secure.enabled);
        secure
            .transition(false, |enabled| {
                calls.push(enabled);
                0
            })
            .unwrap();
        assert_eq!(calls, vec![true, false]);
        assert!(!secure.enabled);
    }
    #[test]
    fn replacement_range_never_confuses_overflow_with_unspecified() {
        assert!(
            replacement_range(Range {
                location: usize::MAX,
                length: 0
            })
            .unwrap()
            .is_none()
        );
        assert!(
            replacement_range(Range {
                location: u32::MAX as usize,
                length: 1
            })
            .is_err()
        );
        assert_eq!(
            replacement_range(Range {
                location: 2,
                length: 3
            })
            .unwrap(),
            Some(TextRange {
                start: 2,
                length: 3
            })
        );
    }
}
