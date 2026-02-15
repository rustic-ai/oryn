# Command Coverage Matrix

Generated from:
- `crates/oryn-core/src/ast.rs`
- `crates/oryn-core/src/parser.rs`
- `crates/oryn-core/src/translator.rs`
- `crates/oryn-engine/src/executor.rs`

Generated at (UTC): `2026-02-13 08:39:10Z`

Regenerate:
- `python scripts/generate-command-coverage-matrix.py`

Legend:
- `implemented`: wired through that stage
- `partial`: wired but behavior is limited
- `stubbed`: placeholder/default or unsupported

Snapshot:
- Total AST commands: `65`
- Parser implemented: `41`
- Parser stubbed: `24`
- Translator implemented: `31`
- Translator stubbed: `34`
- End-to-end implemented: `27`
- Partial in executor/backend path: `3`
- Blocked before execution: `35`

| Command | Parser | Translator | Action | Executor | Note |
|---|---|---|---|---|---|
| Goto | implemented | implemented | `Browser::BrowserAction::Navigate` | implemented | `--headers/--timeout` parse but are currently not applied in translation |
| Back | implemented | implemented | `Browser::BrowserAction::Back` | implemented |  |
| Forward | implemented | implemented | `Browser::BrowserAction::Forward` | implemented |  |
| Refresh | implemented | implemented | `Browser::BrowserAction::Refresh` | implemented | `--hard` parses, but executor currently ignores hard/soft distinction |
| Url | implemented | implemented | `Scanner::ScannerAction::Execute` | implemented |  |
| Observe | implemented | implemented | `Scanner::ScannerAction::Scan` | implemented | `minimal/positions/timeout` parsed but not used in translation |
| Html | implemented | implemented | `Scanner::ScannerAction::GetHtml` | implemented |  |
| Text | implemented | implemented | `Scanner::ScannerAction::GetText` | implemented | `target` parsed but translator uses only `selector` |
| Title | implemented | implemented | `Scanner::ScannerAction::Execute` | implemented |  |
| Screenshot | implemented | implemented | `Browser::BrowserAction::Screenshot` | implemented | `target` parsed but translator sets `selector: None` |
| Box | implemented | stubbed | - | - |  |
| Click | implemented | implemented | `Scanner::ScannerAction::Click` | implemented | `--ctrl/--shift/--alt` parsed but translator sends empty modifiers |
| Type | implemented | implemented | `Scanner::ScannerAction::Type` | implemented | `--append/--timeout` parsed but currently not applied in translation |
| Clear | implemented | implemented | `Scanner::ScannerAction::Clear` | implemented |  |
| Press | implemented | implemented | `Browser::BrowserAction::Press` | implemented |  |
| Keydown | implemented | stubbed | - | - |  |
| Keyup | implemented | stubbed | - | - |  |
| Keys | implemented | stubbed | - | - |  |
| Select | implemented | implemented | `Scanner::ScannerAction::Select` | implemented |  |
| Check | implemented | implemented | `Scanner::ScannerAction::Check` | implemented |  |
| Uncheck | implemented | implemented | `Scanner::ScannerAction::Check` | implemented |  |
| Hover | implemented | implemented | `Scanner::ScannerAction::Hover` | implemented |  |
| Focus | implemented | implemented | `Scanner::ScannerAction::Focus` | implemented |  |
| Scroll | implemented | implemented | `Scanner::ScannerAction::Scroll` | implemented | `--timeout` parsed but currently not applied in translation |
| Submit | implemented | implemented | `Scanner::ScannerAction::Submit` | implemented |  |
| Wait | implemented | implemented | `Scanner::ScannerAction::Wait` | implemented | `wait url "..."` downgraded to generic `navigation`; `ready` maps to unsupported condition |
| Extract | implemented | implemented | `Scanner::ScannerAction::Extract` | implemented |  |
| Cookies | implemented | implemented | `Session::SessionAction::Cookie` | partial | Executor handles list/get/set/delete; `clear` is `NotImplemented`; backend `set_cookie` defaults to `NotSupported` |
| Storage | stubbed | stubbed | - | - |  |
| Sessions | implemented | stubbed | - | - |  |
| Session | stubbed | stubbed | - | - |  |
| State | stubbed | stubbed | - | - |  |
| Headers | stubbed | stubbed | - | - |  |
| Tabs | implemented | implemented | `Browser::BrowserAction::Tab` | partial | Executor handles only tab action `list` |
| Tab | implemented | implemented | `Browser::BrowserAction::Tab` | stubbed | Translator emits `new/switch/close`; executor handles only `list` |
| Login | implemented | implemented | `Scanner::ScannerAction::Login` | implemented | `--no-submit/--wait/--timeout` parsed but current translation ignores these options |
| Search | implemented | implemented | `Scanner::ScannerAction::Search` | implemented | `--submit/--wait/--timeout` parsed but current translation ignores these options |
| Dismiss | implemented | implemented | `Scanner::ScannerAction::Dismiss` | implemented |  |
| AcceptCookies | implemented | implemented | `Scanner::ScannerAction::Accept` | implemented |  |
| ScrollUntil | implemented | stubbed | - | - |  |
| Packs | implemented | stubbed | - | - |  |
| Pack | stubbed | stubbed | - | - |  |
| Intents | stubbed | stubbed | - | - |  |
| Define | stubbed | stubbed | - | - |  |
| Undefine | stubbed | stubbed | - | - |  |
| Export | stubbed | stubbed | - | - |  |
| Run | stubbed | stubbed | - | - |  |
| Intercept | stubbed | stubbed | - | - |  |
| Requests | stubbed | stubbed | - | - |  |
| Console | stubbed | stubbed | - | - |  |
| Errors | stubbed | stubbed | - | - |  |
| Frames | implemented | stubbed | - | - |  |
| Frame | stubbed | stubbed | - | - |  |
| Dialog | stubbed | stubbed | - | - |  |
| Viewport | stubbed | stubbed | - | - |  |
| Device | stubbed | stubbed | - | - |  |
| Devices | implemented | stubbed | - | - |  |
| Media | stubbed | stubbed | - | - |  |
| Trace | stubbed | stubbed | - | - |  |
| Record | stubbed | stubbed | - | - |  |
| Highlight | stubbed | stubbed | - | - |  |
| Pdf | implemented | implemented | `Browser::BrowserAction::Pdf` | partial | Only headless backend implements `pdf` |
| Learn | stubbed | stubbed | - | - |  |
| Exit | implemented | stubbed | - | - | Not translated; REPL exits via raw `exit`/`quit` checks in CLI |
| Help | stubbed | stubbed | - | - |  |
