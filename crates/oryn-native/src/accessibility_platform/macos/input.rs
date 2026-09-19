//! Actual AppKit event → authenticated native input. Never a page-script hook.
use super::*;
use crate::webdriver_actions::{
    Action as Input, ActionsRequest, Origin, OsKey, OsKeyModifiers, PointerKind, PointerProperties,
    Source, SourceKind,
};
unsafe fn unsigned(event: Id, name: &str) -> u64 {
    let f: unsafe extern "C" fn(Id, Sel) -> u64 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(event, sel(name)) }
}
unsafe fn short(event: Id, name: &str) -> u16 {
    let f: unsafe extern "C" fn(Id, Sel) -> u16 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(event, sel(name)) }
}
unsafe fn double(event: Id, name: &str) -> f64 {
    let f: unsafe extern "C" fn(Id, Sel) -> f64 =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe { f(event, sel(name)) }
}
unsafe fn units(string: Id) -> Result<Vec<u16>, String> {
    if string.is_null() {
        return Ok(Vec::new());
    }
    let length = unsafe { unsigned(string, "length") };
    if length > 128 {
        return Err("native keyboard text exceeds event budget".into());
    }
    let mut units = vec![0; length as usize];
    let f: unsafe extern "C" fn(Id, Sel, *mut u16, Range) =
        unsafe { std::mem::transmute(objc_msgSend as *const ()) };
    unsafe {
        f(
            string,
            sel("getCharacters:range:"),
            units.as_mut_ptr(),
            Range {
                location: 0,
                length: units.len(),
            },
        )
    };
    Ok(units)
}
fn special(code: u16) -> Option<u16> {
    Some(match code {
        36 => 0xe006,
        76 => 0xe007,
        53 => 0xe00c,
        48 => 0xe004,
        51 => 0xe003,
        117 => 0xe017,
        123 => 0xe012,
        124 => 0xe014,
        125 => 0xe015,
        126 => 0xe013,
        115 => 0xe011,
        119 => 0xe010,
        116 => 0xe00e,
        121 => 0xe00f,
        122 => 0xe031,
        120 => 0xe032,
        99 => 0xe033,
        118 => 0xe034,
        96 => 0xe035,
        97 => 0xe036,
        98 => 0xe037,
        100 => 0xe038,
        101 => 0xe039,
        109 => 0xe03a,
        103 => 0xe03b,
        111 => 0xe03c,
        _ => return None,
    })
}
fn physical(code: u16) -> (&'static str, u32) {
    match code {
        0 => ("KeyA", 0),
        1 => ("KeyS", 0),
        2 => ("KeyD", 0),
        3 => ("KeyF", 0),
        4 => ("KeyH", 0),
        5 => ("KeyG", 0),
        6 => ("KeyZ", 0),
        7 => ("KeyX", 0),
        8 => ("KeyC", 0),
        9 => ("KeyV", 0),
        11 => ("KeyB", 0),
        12 => ("KeyQ", 0),
        13 => ("KeyW", 0),
        14 => ("KeyE", 0),
        15 => ("KeyR", 0),
        16 => ("KeyY", 0),
        17 => ("KeyT", 0),
        31 => ("KeyO", 0),
        32 => ("KeyU", 0),
        34 => ("KeyI", 0),
        35 => ("KeyP", 0),
        37 => ("KeyL", 0),
        38 => ("KeyJ", 0),
        40 => ("KeyK", 0),
        45 => ("KeyN", 0),
        46 => ("KeyM", 0),
        18 => ("Digit1", 0),
        19 => ("Digit2", 0),
        20 => ("Digit3", 0),
        21 => ("Digit4", 0),
        23 => ("Digit5", 0),
        22 => ("Digit6", 0),
        26 => ("Digit7", 0),
        28 => ("Digit8", 0),
        25 => ("Digit9", 0),
        29 => ("Digit0", 0),
        24 => ("Equal", 0),
        27 => ("Minus", 0),
        30 => ("BracketRight", 0),
        33 => ("BracketLeft", 0),
        39 => ("Quote", 0),
        41 => ("Semicolon", 0),
        42 => ("Backslash", 0),
        43 => ("Comma", 0),
        44 => ("Slash", 0),
        47 => ("Period", 0),
        50 => ("Backquote", 0),
        36 => ("Enter", 0),
        76 => ("NumpadEnter", 3),
        48 => ("Tab", 0),
        49 => ("Space", 0),
        51 => ("Backspace", 0),
        53 => ("Escape", 0),
        117 => ("Delete", 0),
        123 => ("ArrowLeft", 0),
        124 => ("ArrowRight", 0),
        125 => ("ArrowDown", 0),
        126 => ("ArrowUp", 0),
        115 => ("Home", 0),
        119 => ("End", 0),
        116 => ("PageUp", 0),
        121 => ("PageDown", 0),
        56 => ("ShiftLeft", 1),
        60 => ("ShiftRight", 2),
        59 => ("ControlLeft", 1),
        62 => ("ControlRight", 2),
        58 => ("AltLeft", 1),
        61 => ("AltRight", 2),
        55 => ("MetaLeft", 1),
        54 => ("MetaRight", 2),
        57 => ("CapsLock", 0),
        63 => ("Fn", 0),
        122 => ("F1", 0),
        120 => ("F2", 0),
        99 => ("F3", 0),
        118 => ("F4", 0),
        96 => ("F5", 0),
        97 => ("F6", 0),
        98 => ("F7", 0),
        100 => ("F8", 0),
        101 => ("F9", 0),
        109 => ("F10", 0),
        103 => ("F11", 0),
        111 => ("F12", 0),
        82 => ("Numpad0", 3),
        83 => ("Numpad1", 3),
        84 => ("Numpad2", 3),
        85 => ("Numpad3", 3),
        86 => ("Numpad4", 3),
        87 => ("Numpad5", 3),
        88 => ("Numpad6", 3),
        89 => ("Numpad7", 3),
        91 => ("Numpad8", 3),
        92 => ("Numpad9", 3),
        65 => ("NumpadDecimal", 3),
        67 => ("NumpadMultiply", 3),
        69 => ("NumpadAdd", 3),
        75 => ("NumpadDivide", 3),
        78 => ("NumpadSubtract", 3),
        81 => ("NumpadEqual", 3),
        _ => ("", 0),
    }
}
fn native_modifiers(flags: u64) -> OsKeyModifiers {
    OsKeyModifiers {
        shift: flags & (1 << 17) != 0,
        control: flags & (1 << 18) != 0,
        alt: flags & (1 << 19) != 0,
        meta: flags & (1 << 20) != 0,
        caps_lock: flags & (1 << 16) != 0,
        alt_graph: false,
    }
}
fn modifier_key(code: u16) -> Option<(Vec<u16>, u64)> {
    Some(match code {
        56 => (vec![0xe008], 1 << 17),
        60 => (vec![0xe050], 1 << 17),
        59 => (vec![0xe009], 1 << 18),
        62 => (vec![0xe051], 1 << 18),
        58 => (vec![0xe00a], 1 << 19),
        61 => (vec![0xe052], 1 << 19),
        55 => (vec![0xe03d], 1 << 20),
        54 => (vec![0xe053], 1 << 20),
        57 => ("CapsLock".encode_utf16().collect(), 1 << 16),
        63 => ("Fn".encode_utf16().collect(), 1 << 23),
        _ => return None,
    })
}

impl Host {
    pub(super) fn clear_os_input(&mut self) {
        text_input::clear(self.view);
        self.input_keys.clear();
        self.input_modifiers = 0;
        self.input_wheel = (0., 0.);
        self.input_dirty = false;
    }
    pub(super) fn cancel_os_input(&mut self) -> Result<(), String> {
        if self.input_dirty {
            if let Some((scope, target)) = &self.input_scope {
                self.sender
                    .try_send(ActionRequest {
                        scope: Some(scope.clone()),
                        target: *target,
                        revision: Revision(0),
                        action: Action::CancelInput,
                    })
                    .map_err(|_| String::from("native input cancellation queue unavailable"))?;
            }
        }
        self.clear_os_input();
        Ok(())
    }

    fn queue_input(&mut self, kind: SourceKind, actions: Vec<Input>) -> Result<(), String> {
        let Some((scope, target)) = &self.input_scope else {
            return Ok(());
        };
        let id = match kind {
            SourceKind::Key => "oryn-os-key",
            SourceKind::Pointer(_) => "oryn-os-pointer",
            SourceKind::Wheel => "oryn-os-wheel",
            SourceKind::None => "oryn-os-none",
        };
        let request = ActionsRequest {
            sources: vec![Source {
                id: id.into(),
                kind,
                actions,
            }],
        };
        request
            .validate()
            .map_err(|e| format!("{}: {}", e.code, e.message))?;
        self.sender
            .try_send(ActionRequest {
                scope: Some(scope.clone()),
                target: *target,
                revision: Revision(0),
                action: Action::Input(request),
            })
            .map_err(|_| String::from("native OS input queue overflow or page retirement"))?;
        self.input_dirty = true;
        Ok(())
    }
    fn modifiers(&mut self, flags: u64) -> Result<(), String> {
        if flags == self.input_modifiers {
            return Ok(());
        }
        let modifiers = native_modifiers(flags);
        let mut actions = vec![Input::OsModifiers(modifiers)];
        for code in [56, 60, 59, 62, 58, 61, 55, 54, 63] {
            if let Some((_, mask)) = modifier_key(code) {
                if flags & mask == 0 {
                    if let Some(value) = self.input_keys.remove(&code) {
                        let (physical, location) = physical(code);
                        actions.push(Input::OsKey(OsKey {
                            interpretation: None,
                            composing: false,
                            value,
                            code: physical.into(),
                            location,
                            repeat: false,
                            down: false,
                            modifiers,
                        }));
                    }
                }
            }
        }
        self.queue_input(SourceKind::Key, actions)?;
        self.input_modifiers = flags;
        Ok(())
    }

    pub(super) unsafe fn dispatch_os_event(&mut self, event: Id) -> Result<bool, String> {
        unsafe {
            if object(event, "window") != self.window || self.input_scope.is_none() {
                return Ok(false);
            }
            let kind = unsigned(event, "type");
            if matches!(
                kind,
                1 | 2 | 3 | 4 | 5 | 6 | 7 | 10 | 11 | 12 | 22 | 25 | 26 | 27
            ) && text_input::defer_event(self.view, event)?
            {
                return Ok(true);
            }
            if matches!(kind, 10 | 11 | 12) {
                let flags = unsigned(event, "modifierFlags");
                let code = short(event, "keyCode");
                // Keep native application lifecycle/window shortcuts with AppKit.
                if flags & (1 << 20) != 0 && matches!(code, 12 | 13) {
                    return Ok(false);
                }
                self.input_modifiers = flags;
                let modifiers = native_modifiers(flags);
                let (physical, location) = physical(code);
                if kind == 12 {
                    let Some((value, mask)) = modifier_key(code) else {
                        return Ok(true);
                    };
                    if code == 57 {
                        self.queue_input(
                            SourceKind::Key,
                            vec![
                                Input::OsKey(OsKey {
                                    interpretation: None,
                                    composing: false,
                                    value: value.clone(),
                                    code: physical.into(),
                                    location,
                                    repeat: false,
                                    down: true,
                                    modifiers,
                                }),
                                Input::OsKey(OsKey {
                                    interpretation: None,
                                    composing: false,
                                    value,
                                    code: physical.into(),
                                    location,
                                    repeat: false,
                                    down: false,
                                    modifiers,
                                }),
                            ],
                        )?;
                        return Ok(true);
                    }
                    let down = if self.input_keys.remove(&code).is_some() {
                        false
                    } else if flags & mask != 0 {
                        self.input_keys.insert(code, value.clone());
                        true
                    } else {
                        return Ok(true);
                    };
                    self.queue_input(
                        SourceKind::Key,
                        vec![Input::OsKey(OsKey {
                            interpretation: None,
                            composing: false,
                            value,
                            code: physical.into(),
                            location,
                            repeat: false,
                            down,
                            modifiers,
                        })],
                    )?;
                    return Ok(true);
                }
                let key = if kind == 11 {
                    self.input_keys.remove(&code)
                } else {
                    let value = if let Some(key) = special(code) {
                        vec![key]
                    } else {
                        units(object(
                            event,
                            if modifiers.control || modifiers.meta {
                                "charactersIgnoringModifiers"
                            } else {
                                "characters"
                            },
                        ))?
                    };
                    let value = if value.is_empty()
                        || value.len() > 2
                        || value.len() == 2
                            && !((0xd800..=0xdbff).contains(&value[0])
                                && (0xdc00..=0xdfff).contains(&value[1]))
                    {
                        "Unidentified".encode_utf16().collect()
                    } else {
                        value
                    };
                    Some(self.input_keys.entry(code).or_insert(value).clone())
                };
                if let Some(value) = key {
                    let (interpretation, composing) =
                        if kind == 10 && !modifiers.control && !modifiers.meta {
                            let (token, composing) = text_input::reserve(self.view, event)?;
                            (Some(token), composing)
                        } else {
                            (None, text_input::composing(self.view))
                        };
                    let repeated: unsafe extern "C" fn(Id, Sel) -> i8 =
                        std::mem::transmute(objc_msgSend as *const ());
                    self.queue_input(
                        SourceKind::Key,
                        vec![Input::OsKey(OsKey {
                            interpretation,
                            composing,
                            value,
                            code: physical.into(),
                            location,
                            repeat: kind == 10 && repeated(event, sel("isARepeat")) != 0,
                            down: kind == 10,
                            modifiers,
                        })],
                    )?;
                }
                return Ok(true);
            }
            if !matches!(kind, 1 | 2 | 3 | 4 | 5 | 6 | 7 | 22 | 25 | 26 | 27) {
                return Ok(false);
            }
            let location: unsafe extern "C" fn(Id, Sel) -> Point =
                std::mem::transmute(objc_msgSend as *const ());
            let convert: unsafe extern "C" fn(Id, Sel, Point, Id) -> Point =
                std::mem::transmute(objc_msgSend as *const ());
            let point = convert(
                self.view,
                sel("convertPoint:fromView:"),
                location(event, sel("locationInWindow")),
                std::ptr::null_mut(),
            );
            let x = point.x;
            let y = self.input_height - point.y;
            if !x.is_finite() || !y.is_finite() {
                return Err("invalid native pointer coordinates".into());
            }
            // Captured AppKit drag events legitimately leave the content viewport.
            // The typed OS route preserves these coordinates; ordinary WebDriver input
            // continues to use its stricter viewport-origin validation.
            let screen_convert: unsafe extern "C" fn(Id, Sel, Point) -> Point =
                std::mem::transmute(objc_msgSend as *const ());
            let screen = screen_convert(
                self.window,
                sel("convertPointToScreen:"),
                location(event, sel("locationInWindow")),
            );

            self.modifiers(unsigned(event, "modifierFlags"))?;
            let x = x.round() as i64;
            let y = y.round() as i64;
            if kind == 22 {
                if x < 0 || x as f64 > self.input_width || y < 0 || y as f64 > self.input_height {
                    return Ok(false);
                }
                let dx = double(event, "scrollingDeltaX");
                let dy = double(event, "scrollingDeltaY");
                if !dx.is_finite() || !dy.is_finite() {
                    return Err("invalid native wheel delta".into());
                }
                self.input_wheel.0 -= dx;
                self.input_wheel.1 -= dy;
                let delta_x = self.input_wheel.0.round() as i64;
                let delta_y = self.input_wheel.1.round() as i64;
                self.input_wheel.0 -= delta_x as f64;
                self.input_wheel.1 -= delta_y as f64;
                if delta_x != 0 || delta_y != 0 {
                    self.queue_input(
                        SourceKind::Wheel,
                        vec![Input::Scroll {
                            x,
                            y,
                            delta_x,
                            delta_y,
                            origin: Origin::Viewport,
                            duration_ms: None,
                        }],
                    )?;
                }
                return Ok(true);
            }
            let mut actions = vec![Input::OsPointerMove {
                x: point.x,
                y: self.input_height - point.y,
                screen_x: screen.x,
                screen_y: unsafe { CGDisplayBounds(CGMainDisplayID()) }.size.height - screen.y,
            }];
            if matches!(kind, 1 | 2 | 3 | 4 | 25 | 26) {
                let button = match unsigned(event, "buttonNumber") {
                    0 => 0,
                    1 => 2,
                    2 => 1,
                    n => n,
                };
                actions.push(Input::PointerButton {
                    down: matches!(kind, 1 | 3 | 25),
                    button,
                    properties: PointerProperties::default(),
                });
            }
            self.queue_input(SourceKind::Pointer(PointerKind::Mouse), actions)?;
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_modifier_sides_and_capslock_are_native_metadata() {
        assert_eq!(physical(60), ("ShiftRight", 2));
        assert_eq!(modifier_key(60).unwrap().0, vec![0xe050]);
        assert!(native_modifiers(1 << 16).caps_lock);
        assert!(!native_modifiers(1 << 19).alt_graph);
        assert_eq!(physical(0), ("KeyA", 0));
        assert_eq!(physical(76), ("NumpadEnter", 3));
    }
    #[test]
    fn native_navigation_keycodes_map_to_webdriver_special_keys() {
        assert_eq!(special(36), Some(0xe006));
        assert_eq!(special(76), Some(0xe007));
        assert_eq!(special(123), Some(0xe012));
        assert_eq!(special(117), Some(0xe017));
        assert_eq!(special(0), None);
    }
    #[test]
    fn native_os_source_uses_typed_utf16_without_author_json() {
        let request = ActionsRequest {
            sources: vec![Source {
                id: "oryn-os-key".into(),
                kind: SourceKind::Key,
                actions: vec![
                    Input::Key {
                        down: true,
                        value: vec![0xd83d, 0xde00],
                    },
                    Input::Key {
                        down: false,
                        value: vec![0xd83d, 0xde00],
                    },
                ],
            }],
        };
        assert!(request.validate().is_ok());
    }
}
