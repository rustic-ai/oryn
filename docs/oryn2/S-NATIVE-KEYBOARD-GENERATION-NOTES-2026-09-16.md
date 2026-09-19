# Native keyboard source generation

Source only; no build/test/generator execution or passing claims.

`NativeKeyboardInput` carries UTF-16 key units, code, location, repeat, modifier snapshot and down/up. `dispatch_native_keyboard` resolves the real focus within the selected document. It constructs trusted KeyboardEvent instances with native UIEvent payloads, preserving Unicode and WebDriver special-key normalization without author constructors or getters.

Generated defaults include cancellable keydown/keypress/keyup, native input/beforeinput, control grapheme deletion, contenteditable paragraph/line-break edits, selection navigation, select-all, undo/redo, Tab and radio focus, Enter activation/implicit submission, Space activation on keyup, and native single-use clipboard grants only after uncanceled keyboard default admission. Backend clipboard policy remains mandatory. Callback retirement/document changes suppress further defaults.

Regression sources cover constructor replacement, trusted fields, Unicode deletion, canceled keydown/beforeinput, clipboard grant exclusion, retired realms, Space timing, Tab direction and key normalization.

Remaining explicit defaults: page/viewport keyboard scrolling and PageUp/PageDown selection, number/range/select/date picker stepping, platform-specific modifier mappings for all editing/navigation conventions, composition/IME and dead-key text production, browser-reserved shortcuts, modifier lock-state transitions (host supplies snapshot), and cross-frame sequential focus scheduling. Control undo integration is still limited by the existing shared edit history. Keyboard input is a Rust host API; no author global was added. Native host action scheduling/pressed-key registry remains root-owned.

## Follow-up default generation

Control undo/redo now records bounded native edit snapshots centrally in `edit_native_control`, so typing, clipboard defaults and native editing commands share value/selection restoration. Script value assignment and form reset invalidate corresponding history. Undo restores user-edit/change tracking; histories are bounded per control and per binding state.

Added native number/range and value-level date/time step defaults, range Home/End/PageUp/PageDown, single/multiple-select movement with disabled-option exclusion, shift extension and control focus/toggle, and keyboard scrolling through existing native scroll-chain geometry. Space scrolling occurs only after accepted keypress and excludes button activation. Platform modifier behavior is configured through native `NativeKeyboardPlatform` (defaults to host OS), including Mac Meta commands, Alt word movement/deletion, Meta line deletion/navigation and supported Ctrl editing bindings. No author platform strings are consulted.

`native_keyboard_focus_document` returns native focused document identity only after embedding-descendant validation. Dispatch rejects a foreign focused document. Root has wired live frame/realm resolution and added regression source for trusted child events, canceled defaults and stale/sibling rejection.

Additional regression source covers shared control undo/reset invalidation, range bounds, disabled option navigation and platform authority. Still unexecuted. Remaining gaps are segmented date/time picker UI, select typeahead/page navigation, exact platform granularity/coalescing conventions, PageUp/PageDown editable caret goals, complete beforeinput target ranges for word/line deletion, and IME/dead-key composition production. Value-level date/time stepping is not represented as complete segmented picker behavior. Cross-frame Tab scheduling remains separate from correctly routed focused key dispatch.

Further source additions: select typeahead now follows native option labels with disabled exclusion, bounded-time prefix accumulation and repeated-character cycling; keypress cancellation suppresses its default. Native focused control undo is also exposed through execCommand undo/redo and command-enabled queries. Added word/line control deletion modifier behavior with native inputType and preserved UTF-16 boundaries, plus regression source for word deletion, multiple-select shift/control behavior, leap-day native stepping and canceled select typeahead. Programmatic native history limits include a binding-wide cap. Native focus eligibility fixup runs before key targeting.

Still not represented as complete: segmented date/time picker navigation/opening, select popup rendering or select PageUp/PageDown exact visible-row navigation, editable PageUp/PageDown caret goals, all IME/dead-key production, browser-reserved shortcut integration and exact cross-platform editing coalescing. Word/line beforeinput target-range geometry for contenteditable remains a separate source refinement; the underlying deletion uses native segmentation/selection algorithms.
