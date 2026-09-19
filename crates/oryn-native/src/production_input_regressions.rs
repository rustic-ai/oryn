//! Historical behavior oracles exercised through the authenticated Rust host.
//! No page-callable native input hooks are installed by these tests.
use super::*;
use oryn_js::v8_bindings::{NativeKeyboardInput, NativeKeyboardModifiers};

fn key(page: &mut RustExecutableDocument, value: &str, modifiers: NativeKeyboardModifiers) {
    let root = page
        .automation_realms()
        .into_iter()
        .find(|entry| entry.parent_realm.is_none())
        .unwrap();
    let mut input = NativeKeyboardInput {
        key: value.encode_utf16().collect(),
        modifiers,
        down: true,
        ..Default::default()
    };
    page.dispatch_webdriver_keyboard_in_context(&root, &input)
        .unwrap();
    input.down = false;
    // Re-resolve native focus after keydown defaults (particularly Tab).
    page.dispatch_webdriver_keyboard_in_context(&root, &input)
        .unwrap();
}
fn check(page: &mut RustExecutableDocument, source: &str) {
    assert_eq!(page.evaluate(source).unwrap(), "true", "{source}");
}

#[test]
fn authenticated_radio_navigation_uses_form_tree_order_and_cancelable_activation() {
    let mut page=RustExecutableDocument::load("<form id=f><input type=radio name=g id=a checked><button>skip</button><input type=radio name=g disabled><input type=radio name=g inert><input type=radio name=g style='display:none'></form><input type=radio name=g form=f id=b tabindex=-1><input type=radio name=g form=f id=c tabindex=1><input type=radio name=g id=other>").unwrap();
    page.execute("globalThis.log=[];b.onclick=e=>log.push(['click',e.isTrusted,e.pointerId]);b.oninput=()=>log.push(['input']);b.onchange=()=>log.push(['change']);a.focus()").unwrap();
    key(&mut page, "\u{e014}", Default::default());
    check(
        &mut page,
        "document.activeElement===b&&b.checked&&!a.checked&&JSON.stringify(log)==='[[\"click\",true,-1],[\"input\"],[\"change\"]]' ",
    );
    page.execute("b.addEventListener('keydown',e=>e.preventDefault(),{once:true})")
        .unwrap();
    key(&mut page, "ArrowRight", Default::default());
    check(&mut page, "document.activeElement===b&&b.checked");
    page.execute("c.addEventListener('click',e=>e.preventDefault(),{once:true})")
        .unwrap();
    key(&mut page, "ArrowRight", Default::default());
    check(
        &mut page,
        "document.activeElement===c&&!c.checked&&b.checked",
    );
    key(&mut page, "ArrowRight", Default::default());
    check(&mut page, "document.activeElement===a&&a.checked");
    key(&mut page, "ArrowLeft", Default::default());
    check(&mut page, "document.activeElement===c&&c.checked");
    page.execute("c.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true}))")
        .unwrap();
    check(&mut page, "document.activeElement===c&&c.checked");
}

#[test]
fn authenticated_radio_focus_callback_cannot_activate_a_now_disabled_control() {
    let mut page = RustExecutableDocument::load(
        "<input type=radio name=g id=a checked><input type=radio name=g id=b><input id=other>",
    )
    .unwrap();
    page.execute("globalThis.clicks=0;b.onclick=()=>clicks++;b.onfocus=()=>{b.disabled=true;other.focus()};a.focus()").unwrap();
    key(&mut page, "ArrowRight", Default::default());
    check(
        &mut page,
        "document.activeElement===other&&a.checked&&!b.checked&&clicks===0",
    );
}

#[test]
fn authenticated_tab_retargets_keyup_and_orders_positive_tabindex() {
    let mut page=RustExecutableDocument::load("<input id=z><input id=b tabindex=2><input id=a tabindex=1><input id=disabled disabled><input id=hidden style='display:none'>").unwrap();
    page.execute("globalThis.log=[];document.addEventListener('keyup',e=>log.push([e.target.id,e.isTrusted,e.key]));a.focus()").unwrap();
    key(&mut page, "\u{e004}", Default::default());
    check(
        &mut page,
        "document.activeElement===b&&JSON.stringify(log)==='[[\"b\",true,\"Tab\"]]' ",
    );
    key(&mut page, "Tab", Default::default());
    check(&mut page, "document.activeElement===z");
    key(
        &mut page,
        "Tab",
        NativeKeyboardModifiers {
            shift: true,
            ..Default::default()
        },
    );
    check(&mut page, "document.activeElement===b");
    page.execute("b.addEventListener('keydown',e=>e.preventDefault(),{once:true})")
        .unwrap();
    key(&mut page, "Tab", Default::default());
    check(&mut page, "document.activeElement===b");
}

#[test]
fn authenticated_keyboard_edits_native_focus_inside_closed_nested_shadow_roots() {
    let mut page = RustExecutableDocument::load("<div id=host></div><input id=other>").unwrap();
    page.execute("globalThis.outer=host.attachShadow({mode:'closed'});globalThis.innerHost=document.createElement('div');outer.append(innerHost);globalThis.inner=innerHost.attachShadow({mode:'closed'});globalThis.control=document.createElement('input');inner.append(control);control.value='AB';control.focus();control.setSelectionRange(1,1);globalThis.log=[];control.addEventListener('beforeinput',e=>log.push([e.isTrusted,e.inputType,e.data]));").unwrap();
    key(&mut page, "x", Default::default());
    check(
        &mut page,
        "document.activeElement===host&&control.value==='AxB'&&other.value===''&&JSON.stringify(log)==='[[true,\"insertText\",\"x\"]]' ",
    );
    page.execute("control.addEventListener('beforeinput',e=>e.preventDefault(),{once:true})")
        .unwrap();
    key(&mut page, "Delete", Default::default());
    check(&mut page, "control.value==='AxB'");
}

#[test]
fn authenticated_text_edit_preserves_default_value_and_honors_handler_geometry_changes() {
    let mut page =
        RustExecutableDocument::load("<input id=t value=original maxlength=3><input id=other>")
            .unwrap();
    page.execute("t.value='';t.focus();globalThis.inputs=0;t.oninput=e=>{if(!e.isTrusted)throw Error('untrusted input');inputs++}").unwrap();
    for value in ["a", "b", "c", "d"] {
        key(&mut page, value, Default::default());
    }
    check(
        &mut page,
        "t.value==='abc'&&t.defaultValue==='original'&&t.getAttribute('value')==='original'&&inputs===3",
    );
    page.execute(
        "t.value='';t.addEventListener('beforeinput',()=>t.style.display='none',{once:true})",
    )
    .unwrap();
    key(&mut page, "x", Default::default());
    check(&mut page, "t.value===''&&inputs===3");
}

#[test]
fn authenticated_native_events_ignore_author_keyboard_constructor_and_hook_replacements() {
    let mut page = RustExecutableDocument::load("<input id=t>").unwrap();
    page.execute("globalThis.NativeKeyboard=KeyboardEvent;globalThis.log=[];t.focus();for(const type of ['keydown','keypress','keyup'])t.addEventListener(type,e=>log.push([type,e.isTrusted,e instanceof NativeKeyboard,e.composed]));globalThis.KeyboardEvent=()=>{throw Error('author constructor')};globalThis.__oryn_testdriver_keyboard=()=>{throw Error('author hook')};globalThis.__oryn_dispatch_user_input=()=>{throw Error('author hook')}").unwrap();
    key(&mut page, "a", Default::default());
    check(
        &mut page,
        "t.value==='a'&&JSON.stringify(log)==='[[\"keydown\",true,true,true],[\"keypress\",true,true,true],[\"keyup\",true,true,true]]'",
    );
}

fn pointer(
    page: &mut RustExecutableDocument,
    x: f64,
    y: f64,
    action: oryn_js::v8_bindings::NativePointerAction,
    buttons: u32,
) {
    let root = page
        .automation_realms()
        .into_iter()
        .find(|entry| entry.parent_realm.is_none())
        .unwrap();
    let input = oryn_js::v8_bindings::NativePointerInput {
        x,
        y,
        button: if matches!(
            action,
            oryn_js::v8_bindings::NativePointerAction::Down
                | oryn_js::v8_bindings::NativePointerAction::Up
        ) {
            0
        } else {
            -1
        },
        buttons,
        pressure: if buttons == 0 { 0. } else { 0.5 },
        ..Default::default()
    };
    page.dispatch_webdriver_pointer_in_context(&root, &input, action)
        .unwrap();
}
#[test]
fn authenticated_pointer_boundary_order_and_common_ancestor_click_match_native_targets() {
    use oryn_js::v8_bindings::NativePointerAction::*;
    let mut page=RustExecutableDocument::load("<body style='margin:0'><section id=p style='width:200px;height:100px;display:flex'><button id=a style='width:100px;height:100px'></button><button id=b style='width:100px;height:100px'></button></section>").unwrap();
    page.execute("globalThis.boundaries=[];globalThis.clicks=[];for(const n of [p,a,b])for(const type of ['pointerenter','pointerleave'])n.addEventListener(type,e=>boundaries.push([n.id,type,e.target===n,e.isTrusted,e.bubbles,e.composed]));p.onclick=e=>clicks.push([e.target.id,e.isTrusted,e instanceof PointerEvent])").unwrap();
    pointer(&mut page, 20., 20., Move, 0);
    check(
        &mut page,
        "JSON.stringify(boundaries)==='[[\"p\",\"pointerenter\",true,true,false,false],[\"a\",\"pointerenter\",true,true,false,false]]'",
    );
    page.execute("boundaries.length=0").unwrap();
    pointer(&mut page, 20., 20., Down, 1);
    pointer(&mut page, 150., 20., Move, 1);
    pointer(&mut page, 150., 20., Up, 0);
    check(
        &mut page,
        "JSON.stringify(boundaries)==='[[\"a\",\"pointerleave\",true,true,false,false],[\"b\",\"pointerenter\",true,true,false,false]]'&&JSON.stringify(clicks)==='[[\"p\",true,true]]'",
    );
}
#[test]
fn authenticated_canceled_pointerdown_resets_compatibility_suppression_on_release() {
    use oryn_js::v8_bindings::NativePointerAction::*;
    let mut page = RustExecutableDocument::load(
        "<body style='margin:0'><button id=t style='width:100px;height:100px'></button>",
    )
    .unwrap();
    page.execute("globalThis.events=[];t.addEventListener('pointerdown',e=>e.preventDefault(),{once:true});for(const type of ['mousedown','mouseup','click'])t.addEventListener(type,e=>events.push([type,e.isTrusted]))").unwrap();
    pointer(&mut page, 20., 20., Move, 0);
    pointer(&mut page, 20., 20., Down, 1);
    pointer(&mut page, 20., 20., Up, 0);
    check(&mut page, "JSON.stringify(events)==='[[\"click\",true]]'");
    page.execute("events.length=0").unwrap();
    pointer(&mut page, 20., 20., Down, 1);
    pointer(&mut page, 20., 20., Up, 0);
    check(
        &mut page,
        "JSON.stringify(events)==='[[\"mousedown\",true],[\"mouseup\",true],[\"click\",true]]'",
    );
}
#[test]
fn authenticated_label_pointer_default_toggles_control_but_inert_label_does_not() {
    use oryn_js::v8_bindings::NativePointerAction::*;
    let mut page=RustExecutableDocument::load("<body style='margin:0'><input id=c type=checkbox style='position:absolute;left:150px'><label id=l for=c style='display:block;width:100px;height:100px'>toggle</label></body>").unwrap();
    page.execute("globalThis.log=[];c.onclick=e=>log.push(['click',e.isTrusted]);c.oninput=()=>log.push(['input']);c.onchange=()=>log.push(['change'])").unwrap();
    pointer(&mut page, 20., 20., Move, 0);
    pointer(&mut page, 20., 20., Down, 1);
    pointer(&mut page, 20., 20., Up, 0);
    assert_eq!(
        page.evaluate("JSON.stringify([c.checked,log])").unwrap(),
        "[true,[[\"click\",true],[\"input\"],[\"change\"]]]"
    );
    check(
        &mut page,
        "c.checked&&JSON.stringify(log)==='[[\"click\",true],[\"input\"],[\"change\"]]'",
    );
    page.execute("l.inert=true;log.length=0").unwrap();
    pointer(&mut page, 20., 20., Down, 1);
    pointer(&mut page, 20., 20., Up, 0);
    check(&mut page, "c.checked&&log.length===0");
}

#[test]
fn authenticated_wheel_chains_remainder_and_obeys_cancel_and_containment() {
    let mut page=RustExecutableDocument::load("<body style='margin:0'><div id=outer style='width:120px;height:100px;overflow-y:scroll'><div style='width:100px;height:500px'><div id=inner style='width:100px;height:100px;overflow-y:scroll;overscroll-behavior-y:contain'><div style='width:100px;height:300px'>Content</div></div></div></div>").unwrap();
    let root = page
        .automation_realms()
        .into_iter()
        .find(|entry| entry.parent_realm.is_none())
        .unwrap();
    let wheel = oryn_js::v8_bindings::NativeWheelInput {
        x: 20.,
        y: 20.,
        delta_y: 250.,
        ..Default::default()
    };
    page.execute("globalThis.wheels=[];inner.addEventListener('wheel',e=>wheels.push([e instanceof WheelEvent,e.isTrusted,e.deltaY,e.deltaMode,e.view===window]));inner.addEventListener('wheel',e=>e.preventDefault(),{once:true,passive:false})").unwrap();
    assert!(
        !page
            .dispatch_webdriver_wheel_in_context(&root, &wheel)
            .unwrap()
    );
    check(&mut page, "inner.scrollTop===0&&outer.scrollTop===0");
    assert!(
        page.dispatch_webdriver_wheel_in_context(&root, &wheel)
            .unwrap()
    );
    check(
        &mut page,
        "inner.scrollTop===200&&outer.scrollTop===0&&wheels.every(e=>JSON.stringify(e)==='[true,true,250,0,true]')",
    );
    page.execute("inner.scrollTop=0;outer.scrollTop=0;inner.style.overscrollBehaviorY='auto'")
        .unwrap();
    assert!(
        page.dispatch_webdriver_wheel_in_context(&root, &wheel)
            .unwrap()
    );
    check(&mut page, "inner.scrollTop===200&&outer.scrollTop===50");
}

#[test]
fn authenticated_focus_modality_updates_outline_without_overriding_author_style() {
    use oryn_js::v8_bindings::NativePointerAction::*;
    let mut page=RustExecutableDocument::load("<body style='margin:0'><button id=b style='position:absolute;left:0;top:0;width:100px;height:100px'></button><input id=t style='position:absolute;left:150px;top:0;width:100px;height:100px'></body>").unwrap();
    pointer(&mut page, 20., 20., Move, 0);
    pointer(&mut page, 20., 20., Down, 1);
    pointer(&mut page, 20., 20., Up, 0);
    check(
        &mut page,
        "document.activeElement===b&&!b.matches(':focus-visible')&&getComputedStyle(b).outlineStyle==='none'",
    );
    key(&mut page, "a", Default::default());
    check(
        &mut page,
        "b.matches(':focus-visible')&&getComputedStyle(b).outlineStyle==='auto'",
    );
    page.execute("b.style.outline='none'").unwrap();
    check(&mut page, "getComputedStyle(b).outlineStyle==='none'");
    pointer(&mut page, 170., 20., Move, 0);
    pointer(&mut page, 170., 20., Down, 1);
    pointer(&mut page, 170., 20., Up, 0);
    check(
        &mut page,
        "document.activeElement===t&&t.matches(':focus-visible')&&getComputedStyle(t).outlineStyle==='auto'",
    );
    page.execute("t.blur()").unwrap();
    check(&mut page, "getComputedStyle(t).outlineStyle==='none'");
}

fn drag_to_target(page: &mut RustExecutableDocument) {
    use oryn_js::v8_bindings::NativePointerAction::*;
    pointer(page, 20., 20., Move, 0);
    pointer(page, 20., 20., Down, 1);
    pointer(page, 160., 20., Move, 1);
    pointer(page, 160., 20., Up, 0);
}
#[test]
fn authenticated_file_drop_preserves_native_bytes_and_event_order() {
    let mut page=RustExecutableDocument::load("<body style='margin:0'><div id=s draggable=true style='position:absolute;left:0;top:0;width:80px;height:40px'>drag</div><input id=t type=file multiple style='position:absolute;left:120px;top:0;width:180px;height:40px'></body>").unwrap();
    page.execute("globalThis.log=[];s.ondragstart=e=>{e.dataTransfer.items.add(new File(['abc'],'a.txt'));e.dataTransfer.items.add(new File(['12345'],'b.txt'))};for(const type of ['input','change'])t.addEventListener(type,e=>log.push([type,e.isTrusted,t.files.length]));s.ondragend=e=>log.push(['end',e.dataTransfer.dropEffect])").unwrap();
    drag_to_target(&mut page);
    check(
        &mut page,
        "JSON.stringify(log.slice(0,2))==='[[\"input\",true,2],[\"change\",true,2]]'&&Array.from(t.files,f=>f.name+':'+f.size).join(',')==='a.txt:3,b.txt:5'",
    );
    // Read actual File bytes through the native asynchronous API, not only size.
    page.execute("globalThis.bytesResult='pending';Promise.all(Array.from(t.files,f=>f.text())).then(values=>bytesResult=values.join('|'))").unwrap();
    assert_eq!(
        page.evaluate("__oryn_drain(true);bytesResult").unwrap(),
        "abc|12345"
    );
}
#[test]
fn authenticated_drop_revalidates_beforeinput_cancellation_and_handler_selection() {
    for canceled in [false, true] {
        let mut page=RustExecutableDocument::load("<body style='margin:0'><div id=s draggable=true style='position:absolute;left:0;top:0;width:80px;height:40px'>drag</div><input id=t value=abc style='position:absolute;left:120px;top:0;width:180px;height:40px'></body>").unwrap();
        page.execute(&format!("globalThis.cancelDrop={canceled};globalThis.log=[];s.ondragstart=e=>{{e.dataTransfer.setData('text','X');e.dataTransfer.effectAllowed='copy'}};s.ondragend=e=>log.push(e.dataTransfer.dropEffect);t.onbeforeinput=e=>{{log.push(e.inputType+':'+e.data+':'+e.isTrusted);t.setSelectionRange(1,2);if(cancelDrop)e.preventDefault()}};t.oninput=e=>log.push('input:'+e.inputType)")).unwrap();
        drag_to_target(&mut page);
        assert_eq!(
            page.evaluate("t.value+'|'+log.join('|')").unwrap(),
            if canceled {
                "abc|insertFromDrop:X:true|none"
            } else {
                "aXc|insertFromDrop:X:true|input:insertFromDrop|copy"
            }
        );
    }
}
#[test]
fn authenticated_drag_escape_preserves_destination_and_ends_without_click() {
    use oryn_js::v8_bindings::NativePointerAction::*;
    let mut page=RustExecutableDocument::load("<body style='margin:0'><div id=s draggable=true style='position:absolute;left:0;top:0;width:80px;height:40px'>drag</div><input id=t value=abc style='position:absolute;left:120px;top:0;width:180px;height:40px'></body>").unwrap();
    page.execute("globalThis.log=[];s.ondragstart=e=>e.dataTransfer.setData('text','X');s.ondragend=e=>log.push('end:'+e.dataTransfer.dropEffect);t.ondrop=()=>log.push('drop');document.addEventListener('click',()=>log.push('click'))").unwrap();
    pointer(&mut page, 20., 20., Move, 0);
    pointer(&mut page, 20., 20., Down, 1);
    pointer(&mut page, 160., 20., Move, 1);
    key(&mut page, "Escape", Default::default());
    pointer(&mut page, 160., 20., Up, 0);
    check(
        &mut page,
        "t.value==='abc'&&JSON.stringify(log)==='[\"end:none\"]'",
    );
}
#[test]
fn authenticated_rich_drop_sanitizes_markup_and_has_one_native_undo_unit() {
    let mut page=RustExecutableDocument::load("<body style='margin:0'><div id=s draggable=true style='position:absolute;left:0;top:0;width:80px;height:40px'>drag</div><div id=t contenteditable style='position:absolute;left:120px;top:0;width:180px;height:40px'>abc</div></body>").unwrap();
    page.execute("globalThis.log=[];globalThis.saved=[];s.ondragstart=e=>{e.dataTransfer.setData('text/plain','X');e.dataTransfer.setData('text/html','<b>X</b><script>globalThis.pwned=true</script>');e.dataTransfer.effectAllowed='copy'};for(const type of ['beforeinput','input'])t.addEventListener(type,e=>{log.push([type,e.isTrusted,e.inputType]);saved.push(e.dataTransfer)})").unwrap();
    drag_to_target(&mut page);
    check(
        &mut page,
        "t.textContent==='abcX'&&t.querySelector('b').textContent==='X'&&!t.querySelector('script')&&!globalThis.pwned&&JSON.stringify(log)==='[[\"beforeinput\",true,\"insertFromDrop\"],[\"input\",true,\"insertFromDrop\"]]'",
    );
    check(
        &mut page,
        "document.execCommand('undo')&&t.textContent==='abc'&&!t.querySelector('b')",
    );
    check(
        &mut page,
        "document.execCommand('redo')&&t.textContent==='abcX'&&t.querySelector('b').textContent==='X'",
    );
}

#[test]
fn authenticated_enter_uses_selection_editing_host_and_design_mode() {
    for (outer, inner, boundary, design, expected) in [
        ("true", "plaintext-only", false, false, "insertParagraph"),
        ("plaintext-only", "true", false, false, "insertLineBreak"),
        ("true", "plaintext-only", true, false, "insertLineBreak"),
        ("plaintext-only", "true", true, false, "insertParagraph"),
        (
            "plaintext-only",
            "plaintext-only",
            false,
            true,
            "insertParagraph",
        ),
        ("true", "plaintext-only", true, true, "insertLineBreak"),
        ("plaintext-only", "true", true, true, "insertParagraph"),
    ] {
        let mut page = RustExecutableDocument::load("<body></body>").unwrap();
        page.execute(&format!("globalThis.host=document.createElement('div');globalThis.nested=document.createElement('div');host.contentEditable='{outer}';nested.contentEditable='{inner}';if({boundary}){{const barrier=document.createElement('div');barrier.contentEditable='false';host.append(barrier);barrier.append(nested)}}else host.append(nested);nested.innerHTML='<p>ab</p>';document.body.append(host);document.designMode={design}?'on':'off';nested.focus();document.getSelection().collapse(nested.firstChild.firstChild,1);globalThis.observed=[];document.addEventListener('beforeinput',e=>{{observed.push([e.inputType,e.isTrusted]);e.preventDefault()}})")).unwrap();
        key(&mut page, "Enter", Default::default());
        assert_eq!(
            page.evaluate("JSON.stringify(observed)").unwrap(),
            format!("[[\"{expected}\",true]]"),
            "{outer}/{inner}/{boundary}/{design}"
        );
        check(&mut page, "nested.innerHTML==='<p>ab</p>'");
        page.execute("observed.length=0").unwrap();
        key(
            &mut page,
            "Enter",
            NativeKeyboardModifiers {
                shift: true,
                ..Default::default()
            },
        );
        check(
            &mut page,
            "JSON.stringify(observed)==='[[\"insertLineBreak\",true]]'&&nested.innerHTML==='<p>ab</p>'",
        );
    }
}

#[test]
fn authenticated_control_edits_commit_change_once_and_keep_selection_tasks_native() {
    let mut page =
        RustExecutableDocument::load("<form id=f><textarea id=t>abc</textarea><input id=i></form>")
            .unwrap();
    page.execute("globalThis.events=[];globalThis.setTimeout=()=>{throw Error('author timer')};t.onselect=e=>events.push('select:'+e.isTrusted+':'+e.bubbles+':'+e.cancelable);t.setSelectionRange(0,3);events.push('sync')").unwrap();
    page.drain_tasks().unwrap();
    assert_eq!(
        page.evaluate("events.join('|')").unwrap(),
        "sync|select:true:true:false"
    );
    page.execute("events=[];t.setRangeText('foo',2,2);events.push('sync')")
        .unwrap();
    page.drain_tasks().unwrap();
    assert_eq!(
        page.evaluate("events.join('|')").unwrap(),
        "sync|select:true:true:false"
    );
    page.execute("globalThis.changes=[];t.onchange=e=>changes.push([e.isTrusted,e.bubbles,e.cancelable]);t.focus();t.select()").unwrap();
    key(&mut page, "Backspace", Default::default());
    check(&mut page, "t.value===''&&changes.length===0");
    page.execute("t.blur();t.blur()").unwrap();
    check(&mut page, "JSON.stringify(changes)==='[[true,true,false]]'");
    page.execute("t.value=null;i.value=null").unwrap();
    check(&mut page, "t.value===''&&i.value===''");
    page.execute("t.value=undefined;t.focus()").unwrap();
    check(&mut page, "t.value==='undefined'");
    key(&mut page, "x", Default::default());
    page.execute("f.reset();t.blur()").unwrap();
    check(&mut page, "changes.length===1&&t.value==='abc'");
}

#[test]
fn authenticated_pointer_device_payloads_use_native_window_and_interface_slots() {
    use oryn_js::v8_bindings::NativePointerAction::*;
    let mut page = RustExecutableDocument::load(
        "<body style='margin:0'><button id=t style='width:100px;height:100px'></button></body>",
    )
    .unwrap();
    page.execute("globalThis.payloads=[];globalThis.PointerClass=PointerEvent;globalThis.MouseClass=MouseEvent;for(const type of ['pointerenter','mouseenter','pointerdown','mousedown','pointerup','mouseup','click'])t.addEventListener(type,e=>payloads.push([type,e.button,e.buttons,e.detail,e.view===window,e.isTrusted,e instanceof PointerClass,e instanceof MouseClass]));globalThis.PointerEvent=function(){throw Error('author PointerEvent')};globalThis.MouseEvent=function(){throw Error('author MouseEvent')}").unwrap();
    pointer(&mut page, 20., 20., Move, 0);
    pointer(&mut page, 20., 20., Down, 1);
    pointer(&mut page, 20., 20., Up, 0);
    assert_eq!(
        page.evaluate("JSON.stringify(payloads)").unwrap(),
        r#"[["pointerenter",-1,0,0,true,true,true,true],["mouseenter",0,0,0,true,true,false,true],["pointerdown",0,1,0,true,true,true,true],["mousedown",0,1,1,true,true,false,true],["pointerup",0,0,0,true,true,true,true],["mouseup",0,0,1,true,true,false,true],["click",0,0,1,true,true,true,true]]"#
    );
}
#[test]
fn authenticated_radio_direction_uses_live_computed_rtl() {
    let mut page=RustExecutableDocument::load("<body style='direction:rtl'><input type=radio name=g id=a><input type=radio name=g id=b><input type=radio name=g id=c></body>").unwrap();
    page.execute("a.focus()").unwrap();
    key(&mut page, "ArrowRight", Default::default());
    check(&mut page, "document.activeElement===c&&c.checked");
    key(&mut page, "ArrowLeft", Default::default());
    check(&mut page, "document.activeElement===a&&a.checked");
    page.execute("document.body.style.direction='ltr'").unwrap();
    key(&mut page, "ArrowRight", Default::default());
    check(&mut page, "document.activeElement===b&&b.checked");
}

#[test]
fn authenticated_user_validity_commits_on_blur_and_reset_updates_native_style() {
    let mut page=RustExecutableDocument::load("<style>input{width:31px}input:user-valid{width:73px}input:user-invalid{width:79px}</style><form id=f><input id=i required><button id=b>Go</button></form>").unwrap();
    page.execute("i.focus();f.onsubmit=e=>e.preventDefault()")
        .unwrap();
    check(
        &mut page,
        "!i.matches(':user-valid')&&!i.matches(':user-invalid')",
    );
    key(&mut page, "x", Default::default());
    check(&mut page, "!i.matches(':user-valid')");
    page.execute("i.blur()").unwrap();
    check(
        &mut page,
        "i.matches(':user-valid')&&getComputedStyle(i).width==='73px'",
    );
    page.execute("i.value=''").unwrap();
    check(
        &mut page,
        "i.matches(':user-invalid')&&getComputedStyle(i).width==='79px'",
    );
    page.execute("f.reset()").unwrap();
    check(
        &mut page,
        "!i.matches(':user-valid')&&!i.matches(':user-invalid')&&getComputedStyle(i).width==='31px'",
    );
    page.execute("f.requestSubmit()").unwrap();
    check(
        &mut page,
        "i.matches(':user-invalid')&&getComputedStyle(i).width==='79px'",
    );
}

#[test]
fn native_number_bad_input_reaches_production_style_and_placeholder_state() {
    let mut page=RustExecutableDocument::load("<style>input{width:31px}input:placeholder-shown{width:47px}input:invalid{width:79px}</style><form id=f><input id=i type=number placeholder=Number></form>").unwrap();
    page.execute("i.focus()").unwrap();
    check(&mut page, "getComputedStyle(i).width==='47px'");
    key(&mut page, "1", Default::default());
    key(&mut page, "e", Default::default());
    check(
        &mut page,
        "i.value===''&&i.validity.badInput&&!i.matches(':placeholder-shown')&&getComputedStyle(i).width==='79px'",
    );
    key(&mut page, "2", Default::default());
    check(
        &mut page,
        "i.valueAsNumber===100&&!i.validity.badInput&&getComputedStyle(i).width==='31px'",
    );
    page.execute("f.reset()").unwrap();
    check(
        &mut page,
        "!i.validity.badInput&&getComputedStyle(i).width==='47px'",
    );
}

#[test]
fn native_media_backend_and_stylesheet_script_applicability_use_native_state() {
    let mut page=RustExecutableDocument::load("<style id=s media=print>body{width:31px}</style><style>@media (prefers-reduced-motion:reduce){body{width:73px}}</style><body>Media</body>").unwrap();
    page.install_media("dark", "reduce").unwrap();
    check(
        &mut page,
        "matchMedia('(prefers-color-scheme: dark) and (prefers-reduced-motion: reduce)').matches&&!matchMedia('print').matches&&!matchMedia('(invalid-feature)').matches&&getComputedStyle(document.body).width==='73px'",
    );
    page.execute("globalThis.__oryn_media={color_scheme:'light',reduced_motion:'no_preference'};globalThis.__oryn_match_media=()=>false").unwrap();
    check(
        &mut page,
        "matchMedia('(prefers-color-scheme:dark)').matches",
    );
    let style = page
        .agent
        .dom
        .borrow()
        .query_selector(page.agent.dom.borrow().document(), "#s")
        .unwrap()
        .unwrap();
    assert!(
        !page
            .agent
            .host
            .native_stylesheet_blocks_scripts(style)
            .unwrap()
    );
    page.execute("s.media='screen'").unwrap();
    assert!(
        page.agent
            .host
            .native_stylesheet_blocks_scripts(style)
            .unwrap()
    );
    page.execute("s.sheet.disabled=true").unwrap();
    assert!(
        !page
            .agent
            .host
            .native_stylesheet_blocks_scripts(style)
            .unwrap()
    );
    page.execute("s.sheet.disabled=false").unwrap();
    assert!(
        page.agent
            .host
            .native_stylesheet_blocks_scripts(style)
            .unwrap()
    );
    page.execute("s.setAttribute('disabled','')").unwrap();
    assert!(
        !page
            .agent
            .host
            .native_stylesheet_blocks_scripts(style)
            .unwrap()
    );
    page.execute("s.removeAttribute('disabled');s.media='screen'")
        .unwrap();
    assert!(
        page.agent
            .host
            .native_stylesheet_blocks_scripts(style)
            .unwrap()
    );
    page.execute("globalThis.savedStyle=s;s.remove()").unwrap();
    assert!(
        !page
            .agent
            .host
            .native_stylesheet_blocks_scripts(style)
            .unwrap()
    );
    let alternate = {
        let mut dom = page.agent.dom.borrow_mut();
        let node = dom.create_element("link");
        dom.set_attribute(node, "rel", "alternate stylesheet")
            .unwrap();
        dom.set_attribute(node, "href", "/alternate.css").unwrap();
        dom.set_attribute(node, "title", "Other").unwrap();
        let root = dom.document();
        let body = dom.query_selector(root, "body").unwrap().unwrap();
        dom.append_child(body, node).unwrap();
        node
    };
    assert!(
        !page
            .agent
            .host
            .native_stylesheet_blocks_scripts(alternate)
            .unwrap()
    );
    page.agent
        .dom
        .borrow_mut()
        .set_stylesheet_disabled(alternate, false)
        .unwrap();
    assert!(
        page.agent
            .host
            .native_stylesheet_blocks_scripts(alternate)
            .unwrap()
    );
}

#[test]
fn named_stylesheet_sets_control_native_layout_and_parser_applicability() {
    let mut page=RustExecutableDocument::load("<style>body{width:11px}</style><style id=a title=Day>body{width:31px}</style><style id=b title=Night>body{width:73px}</style><body>Sets</body>").unwrap();
    check(
        &mut page,
        "document.preferredStyleSheetSet==='Day'&&document.selectedStyleSheetSet==='Day'&&document.lastStyleSheetSet===null&&getComputedStyle(document.body).width==='31px'",
    );
    page.execute("globalThis.sets=document.styleSheetSets")
        .unwrap();
    check(
        &mut page,
        "sets===document.styleSheetSets&&sets instanceof DOMStringList&&sets.length===2&&sets[0]==='Day'&&sets.item(1)==='Night'&&sets.contains('Night')&&sets.item(9)===null",
    );
    page.execute("document.selectedStyleSheetSet='Night'")
        .unwrap();
    check(
        &mut page,
        "document.lastStyleSheetSet==='Night'&&document.selectedStyleSheetSet==='Night'&&a.sheet.disabled&&!b.sheet.disabled&&getComputedStyle(document.body).width==='73px'",
    );
    let a = page.agent.dom.borrow().find_by_id("a").unwrap();
    let b = page.agent.dom.borrow().find_by_id("b").unwrap();
    assert!(!page.agent.host.native_stylesheet_blocks_scripts(a).unwrap());
    assert!(page.agent.host.native_stylesheet_blocks_scripts(b).unwrap());
    page.execute("document.enableStyleSheetsForSet('Day')")
        .unwrap();
    check(
        &mut page,
        "document.lastStyleSheetSet==='Night'&&document.selectedStyleSheetSet==='Day'&&getComputedStyle(document.body).width==='31px'",
    );
    page.execute("b.sheet.disabled=false").unwrap();
    check(&mut page, "document.selectedStyleSheetSet===null");
    page.execute("document.selectedStyleSheetSet=''").unwrap();
    check(
        &mut page,
        "document.selectedStyleSheetSet===''&&getComputedStyle(document.body).width==='11px'",
    );
    page.execute("document.selectedStyleSheetSet=null").unwrap();
    check(&mut page, "document.lastStyleSheetSet===''");
    page.execute("const extra=document.createElement('style');extra.setAttribute('title','Extra');document.head.append(extra);globalThis.extraSheet=extra").unwrap();
    check(
        &mut page,
        "sets.length===3&&sets[2]==='Extra'&&DOMStringList.prototype.item.call(sets,2)==='Extra'",
    );
    page.execute("extraSheet.remove()").unwrap();
    check(&mut page, "sets.length===2&&!sets.contains('Extra')");
}

#[test]
fn native_media_query_lists_have_identity_shared_listeners_and_change_events() {
    let mut page = RustExecutableDocument::load("<body>Media</body>").unwrap();
    page.execute("globalThis.m=matchMedia('(prefers-color-scheme: dark)');globalThis.events=[];globalThis.listener=function(e){events.push(e.matches+':'+(e.target===m)+':'+e.isTrusted)};m.addListener(listener);m.addEventListener('change',listener);m.onchange=e=>events.push(e.media);globalThis.second=matchMedia('(prefers-color-scheme: dark)')").unwrap();
    check(
        &mut page,
        "m instanceof MediaQueryList&&m instanceof EventTarget&&m!==second&&!m.matches&&events.length===0",
    );
    page.install_media("dark", "reduce").unwrap();
    page.agent.host.drain_scheduled_timers(false).unwrap();
    check(
        &mut page,
        "m.matches&&events.length===2&&events[0]==='true:true:true'&&events[1]===m.media",
    );
    page.agent.host.drain_scheduled_timers(false).unwrap();
    check(&mut page, "events.length===2");
    page.execute("m.removeListener(listener);m.onchange=null")
        .unwrap();
    page.install_media("light", "no_preference").unwrap();
    page.agent.host.drain_scheduled_timers(false).unwrap();
    check(&mut page, "!m.matches&&events.length===2");
    check(
        &mut page,
        "(()=>{const e=new MediaQueryListEvent('change',{media:'screen',matches:true});return e.media==='screen'&&e.matches&&!e.isTrusted&&e instanceof Event})()",
    );
}

#[test]
fn segmented_temporal_keyboard_validation_style_and_semantic_replacement() {
    let mut page=RustExecutableDocument::load("<style>input{width:31px}input:invalid{width:79px}input:user-invalid{width:83px}</style><form id=f><input id=d type=date required><button id=b type=button>Next</button></form>").unwrap();
    page.execute("d.focus();globalThis.edits=0;d.oninput=()=>edits++;d.addEventListener('beforeinput',e=>e.preventDefault(),{once:true})").unwrap();
    key(&mut page, "2", Default::default());
    check(&mut page, "d.value===''&&!d.validity.badInput&&edits===0");
    for value in "20230229".chars() {
        key(&mut page, &value.to_string(), Default::default());
    }
    check(
        &mut page,
        "d.value===''&&d.validity.badInput&&!d.validity.valid&&d.selectionStart===null&&getComputedStyle(d).width==='79px'",
    );
    key(&mut page, "Tab", Default::default());
    check(
        &mut page,
        "document.activeElement===b&&d.matches(':user-invalid')&&getComputedStyle(d).width==='83px'",
    );
    page.execute("f.reset();d.focus()").unwrap();
    for value in "20240229".chars() {
        key(&mut page, &value.to_string(), Default::default());
    }
    check(
        &mut page,
        "d.value==='2024-02-29'&&!d.validity.badInput&&d.validity.valid&&getComputedStyle(d).width==='31px'",
    );
    let node = page.agent.dom.borrow().find_by_id("d").unwrap();
    page.apply_action(node, SemanticAction::Type, Some("2026-09-16"))
        .unwrap();
    check(
        &mut page,
        "d.value==='2026-09-16'&&d.matches(':user-valid')",
    );
    page.execute("d.addEventListener('beforeinput',e=>e.preventDefault(),{once:true})")
        .unwrap();
    assert!(
        page.apply_action(node, SemanticAction::Type, Some("2026-09-17"))
            .is_err()
    );
    check(&mut page, "d.value==='2026-09-16'");
    page.apply_action(node, SemanticAction::Clear, None)
        .unwrap();
    check(&mut page, "d.value===''&&!d.validity.badInput");
}

#[test]
fn all_temporal_control_families_accept_native_field_entry() {
    let mut page=RustExecutableDocument::load("<input id=m type=month><input id=w type=week><input id=t type=time step=0.001><input id=l type=datetime-local>").unwrap();
    for (id, keys, value) in [
        ("m", "2026-09", "2026-09"),
        ("w", "2020-W53", "2020-W53"),
        ("t", "23:59:59.125", "23:59:59.125"),
        ("l", "2026-09-16T12:30", "2026-09-16T12:30"),
    ] {
        page.execute(&format!("document.getElementById('{id}').focus()"))
            .unwrap();
        for key_value in keys.chars() {
            key(&mut page, &key_value.to_string(), Default::default());
        }
        check(
            &mut page,
            &format!(
                "document.getElementById('{id}').value==='{value}'&&!document.getElementById('{id}').validity.badInput"
            ),
        );
    }
}

#[test]
fn temporal_ax_text_uses_rendered_fields_and_selection_drives_native_editing() {
    use oryn_js::accessibility::TextCommand;
    let mut page =
        RustExecutableDocument::load("<input id=d type=date value='2023-02-28'>").unwrap();
    let node = page.agent.dom.borrow().find_by_id("d").unwrap();
    let before = page.accessibility_text(node, &TextCommand::Read).unwrap();
    assert_eq!(String::from_utf16(&before.value).unwrap(), "2023-02-28");
    assert!(!before.boxes.is_empty());
    assert!(!before.carets.is_empty());
    page.accessibility_text(
        node,
        &TextCommand::Select {
            start: 0,
            end: 4,
            expected_value: before.value,
            expected_selection_revision: before.selection_revision,
        },
    )
    .unwrap();
    for value in "2024".chars() {
        key(&mut page, &value.to_string(), Default::default());
    }
    check(&mut page, "d.value==='2024-02-28'&&d.selectionStart===null");
    let after = page.accessibility_text(node, &TextCommand::Read).unwrap();
    assert_eq!((after.start, after.end), (5, 7));
    assert_eq!(String::from_utf16(&after.value).unwrap(), "2024-02-28");
}

#[test]
fn temporal_picker_native_keyboard_constraints_dismissal_and_paint() {
    let mut page=RustExecutableDocument::load("<style>input{width:150px;height:28px}input:open{color:rgb(255,0,0)}</style><form id=f><input id=d lang=fr type=date value=2024-02-28 min=2024-02-01 max=2024-02-29><button id=b type=button>Next</button></form>").unwrap();
    page.execute("d.focus();globalThis.changes=[];d.oninput=e=>changes.push(['input',e.isTrusted]);d.onchange=e=>changes.push(['change',e.isTrusted]);d.click()").unwrap();
    check(&mut page, "!d.matches(':open')");
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    check(
        &mut page,
        "d.matches(':open')&&getComputedStyle(d).color==='rgb(255, 0, 0)'",
    );
    let provider = LiveLayoutProvider {
        style_engine: RefCell::new(Default::default()),
        animation_time: Cell::new(0.),
        animation_revision: Cell::new(0),
        font_revision: Cell::new(0),
        layout_generation: Cell::new(0),
        layout_environment: RefCell::new(None),
        fonts: RefCell::new(Vec::new()),
        owner_thread: std::thread::current().id(),
        paint_inventory: RefCell::new(None),
        dom: page.agent.dom.clone(),
        document: page.agent.dom.borrow().document(),
        viewport: Cell::new(Default::default()),
        scrollbar_width: Cell::new(0.),
        native_media: Cell::new((false, false)),
    };
    let layout = provider.compute_layout(&BTreeMap::new()).unwrap();
    assert!(
        layout
            .generated_nodes
            .iter()
            .any(|node| node.tag == "::picker(temporal)")
    );
    assert!(
        layout
            .generated_nodes
            .iter()
            .any(|node| node.tag == "::temporal-title" && !node.text.is_empty())
    );
    key(&mut page, "ArrowRight", Default::default());
    check(&mut page, "d.value==='2024-02-28'&&changes.length===0");
    page.execute("d.addEventListener('beforeinput',()=>d.max='2024-02-28',{once:true})")
        .unwrap();
    key(&mut page, "Enter", Default::default());
    check(&mut page, "d.value==='2024-02-28'&&changes.length===0");
    page.execute("d.max='2024-02-29'").unwrap();
    key(&mut page, "Enter", Default::default());
    check(
        &mut page,
        "d.value==='2024-02-29'&&!d.matches(':open')&&JSON.stringify(changes)==='[[\"input\",true],[\"change\",true]]'",
    );
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    key(&mut page, "PageUp", Default::default());
    key(&mut page, "Escape", Default::default());
    check(&mut page, "d.value==='2024-02-29'&&!d.matches(':open')");
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    page.execute("f.reset()").unwrap();
    check(&mut page, "!d.matches(':open')&&d.value==='2024-02-28'");
}

#[test]
fn temporal_picker_pointer_uses_private_boxes_and_rechecks_native_candidates() {
    use oryn_js::v8_bindings::NativePointerAction::{Down, Move, Up};
    let mut page = RustExecutableDocument::load(
        "<input id=d type=date value=2024-02-28 style='width:150px;height:28px'>",
    )
    .unwrap();
    page.execute("d.focus()").unwrap();
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    let d = page.agent.dom.borrow().find_by_id("d").unwrap();
    let picker = page
        .agent
        .dom
        .borrow()
        .temporal_picker_snapshot(d)
        .unwrap()
        .unwrap();
    let index = picker
        .items
        .iter()
        .position(|item| item.value == "2024-02-29")
        .unwrap();
    let provider = LiveLayoutProvider {
        style_engine: RefCell::new(Default::default()),
        animation_time: Cell::new(0.),
        animation_revision: Cell::new(0),
        font_revision: Cell::new(0),
        layout_generation: Cell::new(0),
        layout_environment: RefCell::new(None),
        fonts: RefCell::new(Vec::new()),
        owner_thread: std::thread::current().id(),
        paint_inventory: RefCell::new(None),
        dom: page.agent.dom.clone(),
        document: page.agent.dom.borrow().document(),
        viewport: Cell::new(Default::default()),
        scrollbar_width: Cell::new(0.),
        native_media: Cell::new((false, false)),
    };
    let layout = provider.compute_layout(&BTreeMap::new()).unwrap();
    let rect = layout
        .generated_nodes
        .iter()
        .find(|node| node.tag == format!("::temporal-item-{index}"))
        .unwrap()
        .geometry
        .box_model
        .as_ref()
        .unwrap()
        .border_box;
    pointer(
        &mut page,
        rect.x + rect.width / 2.,
        rect.y + rect.height / 2.,
        Move,
        0,
    );
    pointer(
        &mut page,
        rect.x + rect.width / 2.,
        rect.y + rect.height / 2.,
        Down,
        1,
    );
    pointer(
        &mut page,
        rect.x + rect.width / 2.,
        rect.y + rect.height / 2.,
        Up,
        0,
    );
    check(
        &mut page,
        "d.value==='2024-02-29'&&!d.matches(':open')&&document.querySelector('oryn-native-temporal-picker')===null",
    );
}

#[test]
fn localized_temporal_fields_keep_iso_values_through_native_keys_and_oil() {
    use oryn_js::accessibility::TextCommand;
    for (lang, keys, display) in [
        ("en-US", "02292024", "02/29/2024"),
        ("fr", "29022024", "29/02/2024"),
        ("de", "29022024", "29.02.2024"),
    ] {
        let mut page =
            RustExecutableDocument::load(&format!("<input id=d lang='{lang}' type=date>")).unwrap();
        page.execute("d.focus()").unwrap();
        for digit in keys.chars() {
            key(&mut page, &digit.to_string(), Default::default());
        }
        check(&mut page, "d.value==='2024-02-29'");
        let d = page.agent.dom.borrow().find_by_id("d").unwrap();
        let text = page.accessibility_text(d, &TextCommand::Read).unwrap();
        assert_eq!(String::from_utf16(&text.value).unwrap(), display);
        page.apply_action(d, SemanticAction::Type, Some("2026-09-16"))
            .unwrap();
        check(&mut page, "d.value==='2026-09-16'");
    }
}

#[test]
fn input_show_picker_uses_native_activation_and_mutability_checks() {
    let mut page = RustExecutableDocument::load("<input id=d type=date value=2024-02-28>").unwrap();
    check(
        &mut page,
        "(()=>{try{d.showPicker();return false}catch(e){return e.name==='NotAllowedError'}})()",
    );
    page.execute("globalThis.pickerResult='not-called';d.focus();d.addEventListener('keydown',()=>{try{d.showPicker();pickerResult='opened'}catch(e){pickerResult=e.name+':'+e.message;throw e}},{once:true})").unwrap();
    key(&mut page, "F2", Default::default());
    assert_eq!(page.evaluate("pickerResult").unwrap(), "opened");
    check(&mut page, "d.matches(':open')");
    check(
        &mut page,
        "(()=>{try{d.showPicker();return false}catch(e){return e.name==='NotAllowedError'}})()",
    );
    page.execute("d.readOnly=true").unwrap();
    check(
        &mut page,
        "(()=>{try{d.showPicker();return false}catch(e){return e.name==='InvalidStateError'}})()",
    );
    check(&mut page, "!d.matches(':open')");
}

#[test]
fn clock_picker_native_accessibility_actions_choose_hours_minutes_and_seconds_atomically() {
    use oryn_js::accessibility::{PickerAction, TextCommand};
    let mut page =
        RustExecutableDocument::load("<input id=t type=time step=1 value=12:30:45>").unwrap();
    page.execute(
        "t.focus();globalThis.log=[];t.oninput=e=>log.push(e.type);t.onchange=e=>log.push(e.type)",
    )
    .unwrap();
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    let t = page.agent.dom.borrow().find_by_id("t").unwrap();
    let first = page
        .agent
        .dom
        .borrow()
        .temporal_picker_snapshot(t)
        .unwrap()
        .unwrap();
    page.accessibility_text(
        t,
        &TextCommand::Picker {
            expected: first.clone(),
            action: PickerAction::Choose { index: 13 },
        },
    )
    .unwrap();
    check(&mut page, "t.value==='12:30:45'&&log.length===0");
    assert!(
        page.accessibility_text(
            t,
            &TextCommand::Picker {
                expected: first,
                action: PickerAction::Choose { index: 14 }
            }
        )
        .is_err()
    );
    let minute = page
        .agent
        .dom
        .borrow()
        .temporal_picker_snapshot(t)
        .unwrap()
        .unwrap();
    assert_eq!(minute.clock_field, Some(1));
    page.accessibility_text(
        t,
        &TextCommand::Picker {
            expected: minute,
            action: PickerAction::Choose { index: 42 },
        },
    )
    .unwrap();
    check(&mut page, "t.value==='12:30:45'&&log.length===0");
    let second = page
        .agent
        .dom
        .borrow()
        .temporal_picker_snapshot(t)
        .unwrap()
        .unwrap();
    assert_eq!(second.clock_field, Some(2));
    page.accessibility_text(
        t,
        &TextCommand::Picker {
            expected: second.clone(),
            action: PickerAction::Choose { index: 17 },
        },
    )
    .unwrap();
    check(
        &mut page,
        "t.value==='13:42:17'&&!t.matches(':open')&&JSON.stringify(log)==='[\"input\",\"change\"]'",
    );
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    assert!(
        page.accessibility_text(
            t,
            &TextCommand::Picker {
                expected: second,
                action: PickerAction::Clear
            }
        )
        .is_err()
    );
}

#[test]
fn datetime_picker_calendar_then_clock_retains_draft_until_commit() {
    use oryn_js::accessibility::{PickerAction, TextCommand};
    let mut page =
        RustExecutableDocument::load("<input id=t type=datetime-local value=2024-02-28T12:30>")
            .unwrap();
    page.execute("t.focus()").unwrap();
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    let t = page.agent.dom.borrow().find_by_id("t").unwrap();
    let calendar = page
        .agent
        .dom
        .borrow()
        .temporal_picker_snapshot(t)
        .unwrap()
        .unwrap();
    let index = calendar
        .items
        .iter()
        .position(|item| item.value == "2024-02-29T12:30")
        .unwrap();
    page.accessibility_text(
        t,
        &TextCommand::Picker {
            expected: calendar,
            action: PickerAction::Choose { index },
        },
    )
    .unwrap();
    check(&mut page, "t.value==='2024-02-28T12:30'");
    let hour = page
        .agent
        .dom
        .borrow()
        .temporal_picker_snapshot(t)
        .unwrap()
        .unwrap();
    page.accessibility_text(
        t,
        &TextCommand::Picker {
            expected: hour,
            action: PickerAction::Choose { index: 14 },
        },
    )
    .unwrap();
    let minute = page
        .agent
        .dom
        .borrow()
        .temporal_picker_snapshot(t)
        .unwrap()
        .unwrap();
    page.accessibility_text(
        t,
        &TextCommand::Picker {
            expected: minute,
            action: PickerAction::Choose { index: 35 },
        },
    )
    .unwrap();
    check(&mut page, "t.value==='2024-02-29T14:35'");
}

#[test]
fn rgb_picker_and_datalist_use_native_candidate_actions_without_author_dom_rows() {
    use oryn_js::accessibility::{PickerAction, TextCommand};
    let mut page=RustExecutableDocument::load("<input id=c type=color value=invalid><input id=t list=choices><datalist id=choices><option value=alpha><option value=disabled disabled><option value=beta></datalist>").unwrap();
    check(
        &mut page,
        "c.value==='#000000'&&t.list===choices&&choices instanceof HTMLDataListElement&&choices.options.length===3",
    );
    page.execute("c.focus();globalThis.events=[];c.onchange=()=>events.push('change')")
        .unwrap();
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    let c = page.agent.dom.borrow().find_by_id("c").unwrap();
    for value in [18, 52, 86] {
        let expected = page
            .agent
            .dom
            .borrow()
            .temporal_picker_snapshot(c)
            .unwrap()
            .unwrap();
        page.accessibility_text(
            c,
            &TextCommand::Picker {
                expected,
                action: PickerAction::Choose { index: value },
            },
        )
        .unwrap();
    }
    check(&mut page, "c.value==='#123456'&&events.length===1");
    page.execute("t.focus()").unwrap();
    key(&mut page, "ArrowDown", Default::default());
    let t = page.agent.dom.borrow().find_by_id("t").unwrap();
    let expected = page
        .agent
        .dom
        .borrow()
        .temporal_picker_snapshot(t)
        .unwrap()
        .unwrap();
    assert_eq!(expected.items.len(), 2);
    page.execute("choices.lastElementChild.value='changed'")
        .unwrap();
    assert!(
        page.accessibility_text(
            t,
            &TextCommand::Picker {
                expected,
                action: PickerAction::Choose { index: 1 }
            }
        )
        .is_err()
    );
    let expected = page
        .agent
        .dom
        .borrow()
        .temporal_picker_snapshot(t)
        .unwrap()
        .unwrap();
    page.accessibility_text(
        t,
        &TextCommand::Picker {
            expected,
            action: PickerAction::Choose { index: 1 },
        },
    )
    .unwrap();
    check(&mut page, "t.value==='changed'&&!t.matches(':open')");
}

#[test]
fn native_file_chooser_commits_real_bytes_cancels_and_rejects_retired_sessions() {
    use oryn_js::accessibility::{PickerFile, TextCommand};
    let mut page = RustExecutableDocument::load(
        "<form id=f><input id=u type=file name=upload multiple></form>",
    )
    .unwrap();
    page.execute("u.focus();globalThis.events=[];for(const type of ['input','change','cancel'])u.addEventListener(type,e=>events.push([e.type,e.isTrusted]))").unwrap();
    key(&mut page, "Enter", Default::default());
    let u = page.agent.dom.borrow().find_by_id("u").unwrap();
    let expected = page
        .agent
        .dom
        .borrow()
        .file_picker_snapshot(u)
        .unwrap()
        .unwrap();
    let files = vec![PickerFile {
        name: "first.txt".into(),
        media_type: "text/plain".into(),
        bytes: b"real native bytes".to_vec(),
        last_modified: 123,
    }];
    page.accessibility_text(
        u,
        &TextCommand::FilePicker {
            expected: expected.clone(),
            files: Some(files.clone()),
        },
    )
    .unwrap();
    check(
        &mut page,
        "u.files.length===1&&u.files[0].name==='first.txt'&&u.files[0].size===17&&new FormData(f).get('upload')===u.files[0]&&JSON.stringify(events)==='[[\"input\",true],[\"change\",true]]'",
    );
    assert!(
        page.accessibility_text(
            u,
            &TextCommand::FilePicker {
                expected,
                files: Some(files.clone())
            }
        )
        .is_err()
    );
    key(&mut page, "Enter", Default::default());
    let expected = page
        .agent
        .dom
        .borrow()
        .file_picker_snapshot(u)
        .unwrap()
        .unwrap();
    page.accessibility_text(
        u,
        &TextCommand::FilePicker {
            expected,
            files: None,
        },
    )
    .unwrap();
    check(
        &mut page,
        "u.files[0].name==='first.txt'&&events.at(-1)[0]==='cancel'",
    );
    key(&mut page, "Enter", Default::default());
    let expected = page
        .agent
        .dom
        .borrow()
        .file_picker_snapshot(u)
        .unwrap()
        .unwrap();
    page.execute("f.reset()").unwrap();
    assert!(
        page.accessibility_text(
            u,
            &TextCommand::FilePicker {
                expected,
                files: Some(files)
            }
        )
        .is_err()
    );
    check(&mut page, "u.files.length===0");
}

#[test]
fn range_pointer_drag_uses_native_projection_constraints_and_single_commit() {
    use oryn_js::v8_bindings::NativePointerAction::{Down, Move, Up};
    let mut page=RustExecutableDocument::load("<body style='margin:0'><input id=r type=range min=0 max=100 step=10 value=0 readonly style='position:absolute;left:0;top:0;width:200px;height:40px;margin:0;padding:0;border:0;transform:translateX(40px)'>").unwrap();
    page.execute("globalThis.events=[];r.oninput=e=>events.push(['input',e.isTrusted]);r.onchange=e=>events.push(['change',e.isTrusted])").unwrap();
    pointer(&mut page, 40., 20., Down, 1);
    pointer(&mut page, 140., 20., Move, 1);
    pointer(&mut page, 240., 20., Move, 1);
    pointer(&mut page, 240., 20., Up, 0);
    check(
        &mut page,
        "r.value==='100'&&events.filter(e=>e[0]==='change').length===1&&events.every(e=>e[1])",
    );
    page.execute("r.value=0;r.addEventListener('mousedown',e=>e.preventDefault(),{once:true})")
        .unwrap();
    pointer(&mut page, 140., 20., Down, 1);
    pointer(&mut page, 140., 20., Up, 0);
    check(&mut page, "r.value==='0'");
}

#[test]
fn native_popup_wheel_changes_highlight_without_author_wheel_or_value_commit() {
    let mut page = RustExecutableDocument::load("<input id=c type=color value='#000000'>").unwrap();
    page.execute("c.focus();globalThis.wheels=0;document.addEventListener('wheel',e=>{wheels++;e.preventDefault()},{capture:true,passive:false})").unwrap();
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    let c = page.agent.dom.borrow().find_by_id("c").unwrap();
    let provider = LiveLayoutProvider {
        style_engine: RefCell::new(Default::default()),
        animation_time: Cell::new(0.),
        animation_revision: Cell::new(0),
        font_revision: Cell::new(0),
        layout_generation: Cell::new(0),
        layout_environment: RefCell::new(None),
        fonts: RefCell::new(Vec::new()),
        owner_thread: std::thread::current().id(),
        paint_inventory: RefCell::new(None),
        dom: page.agent.dom.clone(),
        document: page.agent.dom.borrow().document(),
        viewport: Cell::new(Default::default()),
        scrollbar_width: Cell::new(0.),
        native_media: Cell::new((false, false)),
    };
    let paint = provider.compute_layout(&BTreeMap::new()).unwrap();
    let rect = paint
        .generated_nodes
        .iter()
        .find(|node| node.tag == "::picker(temporal)")
        .unwrap()
        .geometry
        .box_model
        .as_ref()
        .unwrap()
        .border_box;
    let root = page
        .automation_realms()
        .into_iter()
        .find(|entry| entry.parent_realm.is_none())
        .unwrap();
    page.dispatch_webdriver_wheel_in_context(
        &root,
        &oryn_js::v8_bindings::NativeWheelInput {
            x: rect.x + 4.,
            y: rect.y + 4.,
            delta_y: 96.,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        page.agent
            .dom
            .borrow()
            .temporal_picker_snapshot(c)
            .unwrap()
            .unwrap()
            .active
            > 0
    );
    check(&mut page, "c.value==='#000000'&&wheels===0");
}

#[test]
fn modern_color_attributes_picker_and_formdata_share_native_color_space() {
    use oryn_js::accessibility::{PickerAction, TextCommand};
    let mut page = RustExecutableDocument::load(
        "<form id=f><input id=c name=color type=color value='#ffffff08'></form>",
    )
    .unwrap();
    check(
        &mut page,
        "c.value==='#ffffff'&&c.colorSpace==='limited-srgb'&&!c.alpha",
    );
    page.execute("c.alpha=true").unwrap();
    check(
        &mut page,
        "c.value.startsWith('color(srgb 1 1 1 / 0.03137')",
    );
    page.execute("c.colorSpace='DISPLAY-P3';c.value='color(display-p3 3 none .2 / .6)'")
        .unwrap();
    check(
        &mut page,
        "c.colorSpace==='display-p3'&&c.value==='color(display-p3 3 0 0.2 / 0.6)'&&new FormData(f).get('color')===c.value",
    );
    page.execute("c.alpha=false;c.alpha=true").unwrap();
    check(&mut page, "c.value==='color(display-p3 3 0 0.2)'");
    page.execute("c.colorSpace='invalid';c.value='transparent';c.focus()")
        .unwrap();
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    let c = page.agent.dom.borrow().find_by_id("c").unwrap();
    for channel in [255, 0, 0, 128] {
        let expected = page
            .agent
            .dom
            .borrow()
            .temporal_picker_snapshot(c)
            .unwrap()
            .unwrap();
        page.accessibility_text(
            c,
            &TextCommand::Picker {
                expected,
                action: PickerAction::Choose { index: channel },
            },
        )
        .unwrap();
    }
    check(
        &mut page,
        "c.value.startsWith('color(srgb 1 0 0 / 0.50196')&&new FormData(f).get('color')===c.value",
    );
}

#[test]
fn continuous_touch_slider_retains_contact_events_and_does_not_pan_page() {
    use oryn_js::v8_bindings::{NativePointerAction, NativePointerDevice, NativePointerInput};
    let mut page=RustExecutableDocument::load("<body style='margin:0;height:1500px'><input id=r type=range min=0 max=100 step=10 value=0 style='position:absolute;left:0;top:0;width:200px;height:40px;margin:0;padding:0;border:0'>").unwrap();
    page.execute("globalThis.touchmoves=0;globalThis.changes=0;r.ontouchmove=e=>{if(e.touches.length===1&&e.changedTouches.length===1)touchmoves++};r.onchange=()=>changes++").unwrap();
    let root = page
        .automation_realms()
        .into_iter()
        .find(|entry| entry.parent_realm.is_none())
        .unwrap();
    for (action, x, buttons) in [
        (NativePointerAction::Down, 8., 1),
        (NativePointerAction::Move, 100., 1),
        (NativePointerAction::Move, 192., 1),
        (NativePointerAction::Up, 192., 0),
    ] {
        let input = NativePointerInput {
            id: 19,
            device: NativePointerDevice::Touch,
            primary: true,
            x,
            y: 20.,
            button: if action == NativePointerAction::Move {
                -1
            } else {
                0
            },
            buttons,
            pressure: if buttons == 0 { 0. } else { 0.5 },
            ..Default::default()
        };
        page.dispatch_webdriver_pointer_in_context(&root, &input, action)
            .unwrap();
    }
    check(
        &mut page,
        "r.value==='100'&&touchmoves===2&&changes===1&&scrollY===0",
    );
}

#[test]
fn native_locale_digits_flow_through_production_keyboard_and_iso_form_values() {
    let mut page =
        RustExecutableDocument::load("<form id=f><input id=d name=date type=date lang=ar></form>")
            .unwrap();
    let locale = oryn_dom::ControlLocale {
        months: std::array::from_fn(|i| format!("month{}", i + 1)),
        weekdays: std::array::from_fn(|i| format!("day{i}")),
        first_weekday: 6,
        date_order: [2, 1, 0],
        separators: ["/".into(), "/".into()],
        digits: ['٠', '١', '٢', '٣', '٤', '٥', '٦', '٧', '٨', '٩'],
    };
    page.agent
        .dom
        .borrow_mut()
        .install_control_locale("ar", locale)
        .unwrap();
    page.execute("d.focus()").unwrap();
    for digit in "٢٩٠٢٢٠٢٤".chars() {
        key(&mut page, &digit.to_string(), Default::default());
    }
    check(
        &mut page,
        "d.value==='2024-02-29'&&new FormData(f).get('date')==='2024-02-29'",
    );
    let d = page.agent.dom.borrow().find_by_id("d").unwrap();
    let text = page
        .accessibility_text(d, &oryn_js::accessibility::TextCommand::Read)
        .unwrap();
    assert_eq!(String::from_utf16(&text.value).unwrap(), "٢٩/٠٢/٢٠٢٤");
}

#[test]
fn semantic_range_and_color_ignore_inapplicable_readonly_and_use_event_input() {
    let mut page = RustExecutableDocument::load(
        "<input id=r type=range readonly min=0 max=10 step=2><input id=c type=color readonly>",
    )
    .unwrap();
    page.execute("globalThis.events=[];for(const node of [r,c])node.oninput=e=>events.push([e instanceof InputEvent,e.isTrusted])").unwrap();
    let r = page.agent.dom.borrow().find_by_id("r").unwrap();
    let c = page.agent.dom.borrow().find_by_id("c").unwrap();
    page.apply_action(r, SemanticAction::Type, Some("7"))
        .unwrap();
    page.apply_action(c, SemanticAction::Type, Some("crimson"))
        .unwrap();
    check(
        &mut page,
        "r.value==='8'&&c.value==='#dc143c'&&JSON.stringify(events)==='[[false,true],[false,true]]'",
    );
}

#[test]
fn native_ax_numeric_adjustment_reuses_step_bounds_and_rejects_readonly_number() {
    use oryn_js::accessibility::TextCommand;
    let mut page=RustExecutableDocument::load("<input id=r type=range min=2 max=8 step=2 value=4 readonly><input id=n type=number value=4 readonly>").unwrap();
    page.execute(
        "globalThis.events=[];r.oninput=e=>events.push(e.type);r.onchange=e=>events.push(e.type)",
    )
    .unwrap();
    let r = page.agent.dom.borrow().find_by_id("r").unwrap();
    let n = page.agent.dom.borrow().find_by_id("n").unwrap();
    page.accessibility_text(r, &TextCommand::Adjust { increase: true })
        .unwrap();
    page.accessibility_text(r, &TextCommand::Adjust { increase: true })
        .unwrap();
    page.accessibility_text(r, &TextCommand::Adjust { increase: true })
        .unwrap();
    check(
        &mut page,
        "r.value==='8'&&events.join(',')==='input,change,input,change'",
    );
    assert!(
        page.accessibility_text(n, &TextCommand::Adjust { increase: true })
            .is_err()
    );
    page.execute("r.disabled=true").unwrap();
    assert!(
        page.accessibility_text(r, &TextCommand::Adjust { increase: false })
            .is_err()
    );
}

#[test]
fn select_picker_scroll_has_native_authority_and_cancelable_wheel_default() {
    fn picker(page: &RustExecutableDocument, scrollbar: f64) -> oryn_common::v4::PaintNodeSnapshot {
        let provider = LiveLayoutProvider {
            style_engine: RefCell::new(Default::default()),
            animation_time: Cell::new(0.),
            animation_revision: Cell::new(0),
            font_revision: Cell::new(0),
            layout_generation: Cell::new(0),
            layout_environment: RefCell::new(None),
            fonts: RefCell::new(Vec::new()),
            owner_thread: std::thread::current().id(),
            paint_inventory: RefCell::new(None),
            dom: page.agent.dom.clone(),
            document: page.agent.dom.borrow().document(),
            viewport: Cell::new(Default::default()),
            scrollbar_width: Cell::new(scrollbar),
            native_media: Cell::new((false, false)),
        };
        provider
            .compute_layout(&BTreeMap::new())
            .unwrap()
            .generated_nodes
            .into_iter()
            .find(|node| node.tag == "::picker(select)")
            .unwrap()
    }
    let options = (0..20)
        .map(|index| format!("<option>Option {index}</option>"))
        .collect::<String>();
    let mut page=RustExecutableDocument::load(&format!("<style>select,select::picker(select){{appearance:base-select}}select::picker(select){{height:80px;overflow:auto;overscroll-behavior:contain}}option{{display:block;height:20px}}</style><select id=s>{options}</select>")).unwrap();
    page.execute("s.focus()").unwrap();
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    key(&mut page, "End", Default::default());
    let shown = picker(&page, 0.);
    let initial = shown
        .projection
        .as_ref()
        .unwrap()
        .scroll
        .as_ref()
        .unwrap()
        .offset_y;
    assert!(initial > 0.);
    check(
        &mut page,
        "s.selectedIndex===0&&s.scrollTop===0&&s.matches(':open')",
    );
    let select = page.agent.dom.borrow().find_by_id("s").unwrap();
    let rect = shown.geometry.box_model.as_ref().unwrap().border_box;
    let root = page
        .automation_realms()
        .into_iter()
        .find(|entry| entry.parent_realm.is_none())
        .unwrap();
    page.execute("s.addEventListener('wheel',e=>e.preventDefault(),{once:true})")
        .unwrap();
    let wheel = oryn_js::v8_bindings::NativeWheelInput {
        x: rect.x + 10.,
        y: rect.y + 10.,
        delta_y: -40.,
        ..Default::default()
    };
    page.dispatch_webdriver_wheel_in_context(&root, &wheel)
        .unwrap();
    assert_eq!(
        picker(&page, 0.)
            .projection
            .as_ref()
            .unwrap()
            .scroll
            .as_ref()
            .unwrap()
            .offset_y,
        initial
    );
    page.dispatch_webdriver_wheel_in_context(&root, &wheel)
        .unwrap();
    let scrolled = picker(&page, 0.)
        .projection
        .as_ref()
        .unwrap()
        .scroll
        .as_ref()
        .unwrap()
        .offset_y;
    assert!(scrolled < initial);
    assert!(
        !page
            .agent
            .dom
            .borrow()
            .top_layer()
            .find(|entry| entry.node == select)
            .unwrap()
            .picker_reveal
    );
    check(
        &mut page,
        "s.scrollTop===0&&window.scrollY===0&&s.selectedIndex===0",
    );
    key(&mut page, "Escape", Default::default());
    key(
        &mut page,
        "ArrowDown",
        NativeKeyboardModifiers {
            alt: true,
            ..Default::default()
        },
    );
    assert_eq!(
        picker(&page, 0.)
            .projection
            .as_ref()
            .unwrap()
            .scroll
            .as_ref()
            .unwrap()
            .offset_y,
        0.
    );
    page.agent.host.set_native_scrollbar_width(12.).unwrap();
    let shown = picker(&page, 12.);
    let projection = shown.projection.as_ref().unwrap();
    let bars = oryn_common::scrollbar::geometry(projection, &shown.style);
    let bar = bars.iter().find(|bar| !bar.horizontal).unwrap();
    let x = bar.thumb.x + bar.thumb.width / 2.;
    let y = bar.thumb.y + bar.thumb.height / 2.;
    use oryn_js::v8_bindings::NativePointerAction::{Down, Move, Up};
    pointer(&mut page, x, y, Move, 0);
    pointer(&mut page, x, y, Down, 1);
    pointer(&mut page, x, y + 20., Move, 1);
    pointer(&mut page, x, y + 20., Up, 0);
    assert!(
        page.agent
            .dom
            .borrow()
            .top_layer()
            .find(|entry| entry.node == select)
            .unwrap()
            .picker_scroll
            .1
            > 0.
    );
    check(
        &mut page,
        "s.scrollTop===0&&s.matches(':open')&&s.selectedIndex===0",
    );
}

#[test]
fn native_maxlength_distinguishes_blocked_insertion_from_same_value_replacement() {
    let mut page = RustExecutableDocument::load("<input id=t value=abc maxlength=3>").unwrap();
    page.execute("t.focus();t.setSelectionRange(3,3);globalThis.inputs=[];t.oninput=e=>inputs.push([e.inputType,e.isTrusted]);").unwrap();
    key(&mut page, "d", Default::default());
    check(&mut page, "t.value==='abc'&&inputs.length===0");
    page.execute("t.setSelectionRange(1,2)").unwrap();
    key(&mut page, "b", Default::default());
    check(
        &mut page,
        "t.value==='abc'&&JSON.stringify(inputs)==='[[\"insertText\",true]]'",
    );
    key(&mut page, "Backspace", Default::default());
    check(
        &mut page,
        "t.value==='ac'&&inputs.length===2&&inputs[1][0]==='deleteContentBackward'",
    );
}

#[test]
fn design_mode_does_not_override_shadow_plaintext_host() {
    let mut page = RustExecutableDocument::load("<div id=h></div>").unwrap();
    page.execute("globalThis.root=h.attachShadow({mode:'open'});globalThis.editor=document.createElement('div');editor.contentEditable='plaintext-only';editor.textContent='ab';root.append(editor);document.designMode='on';editor.focus();document.getSelection().collapse(editor.firstChild,1);globalThis.observed=[];editor.onbeforeinput=e=>{observed.push(e.inputType);e.preventDefault()}").unwrap();
    key(&mut page, "Enter", Default::default());
    check(
        &mut page,
        "JSON.stringify(observed)==='[\"insertLineBreak\"]'&&editor.textContent==='ab'",
    );
}

#[test]
fn beforeinput_inert_callback_prevents_native_control_mutation() {
    let mut page = RustExecutableDocument::load("<input id=t value=abc>").unwrap();
    page.execute("t.focus();t.setSelectionRange(1,2);globalThis.inputs=0;t.oninput=()=>inputs++;t.onbeforeinput=()=>t.inert=true").unwrap();
    key(&mut page, "x", Default::default());
    check(&mut page, "t.value==='abc'&&inputs===0");
}

#[test]
fn standalone_native_activation_propagates_and_consumes_across_frame_tree() {
    let mut page = RustExecutableDocument::load(
        "<input id=root type=date><iframe id=f srcdoc='<input id=child type=date>'></iframe>",
    )
    .unwrap();
    page.execute("globalThis.activation=[];f.contentDocument.getElementById('child').focus();f.contentDocument.addEventListener('keydown',()=>{activation.push([navigator.userActivation.isActive,f.contentWindow.navigator.userActivation.isActive]);root.showPicker();activation.push([navigator.userActivation.isActive,f.contentWindow.navigator.userActivation.isActive])},{once:true})").unwrap();
    key(&mut page, "F2", Default::default());
    check(
        &mut page,
        "JSON.stringify(activation)==='[[true,true],[false,false]]'&&root.matches(':open')",
    );
}
