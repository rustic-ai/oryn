use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Script {
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub command: Option<Command>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    // Navigation
    Goto(GotoCmd),
    Back,
    Forward,
    Refresh(RefreshCmd),
    Url,

    // Observation
    Observe(ObserveCmd),
    Html(HtmlCmd),
    Text(TextCmd),
    Title,
    Screenshot(ScreenshotCmd),
    Box(BoxCmd),

    // Actions
    Click(ClickCmd),
    Type(TypeCmd),
    Clear(ClearCmd),
    Press(PressCmd),
    Keydown(KeydownCmd),
    Keyup(KeyupCmd),
    Keys,
    Select(SelectCmd),
    Check(CheckCmd),
    Uncheck(UncheckCmd),
    Hover(HoverCmd),
    Focus(FocusCmd),
    Scroll(ScrollCmd),
    Submit(SubmitCmd),

    // Wait
    Wait(WaitCmd),

    // Extract
    Extract(ExtractCmd),

    // Session
    Cookies(CookiesCmd),
    Storage(StorageCmd),
    Sessions,
    Session(SessionMgmtCmd),
    State(StateCmd),
    Headers(HeadersCmd),

    // Tabs
    Tabs,
    Tab(TabActionCmd),

    // Intents
    Login(LoginCmd),
    Search(SearchCmd),
    Dismiss(DismissCmd),
    AcceptCookies,
    ScrollUntil(ScrollUntilCmd),

    // Packs
    Packs,
    Pack(PackActionCmd),
    Intents(IntentsCmd),
    Define(DefineCmd),
    Undefine(UndefineCmd),
    Export(ExportCmd),
    Run(RunCmd),

    // Network
    Intercept(InterceptCmd),
    Requests(RequestsCmd),

    // Console
    Console(ConsoleCmd),
    Errors(ErrorsCmd),

    // Frames
    Frames,
    Frame(FrameSwitchCmd),

    // Dialog
    Dialog(DialogCmd),

    // Viewport
    Viewport(ViewportSizeCmd),
    Device(DeviceCmd),
    Devices,
    Media(MediaCmd),

    // Recording
    Trace(TraceCmd),
    Record(RecordCmd),
    Highlight(HighlightCmd),

    // Utility
    Pdf(PdfCmd),
    Learn(LearnCmd),
    Exit,
    Help(HelpCmd),
}

// --- Navigation ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GotoCmd {
    pub url: String,
    pub headers: Option<String>,
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefreshCmd {
    pub hard: bool,
}

// --- Observation ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObserveCmd {
    pub full: bool,
    pub minimal: bool,
    pub viewport: bool,
    pub hidden: bool,
    pub positions: bool,
    pub diff: bool,
    pub near: Option<String>,
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HtmlCmd {
    pub selector: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextCmd {
    pub selector: Option<String>,
    pub target: Option<Target>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenshotCmd {
    pub output: Option<String>,
    pub format: Option<String>,
    pub fullpage: bool,
    pub target: Option<Target>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoxCmd {
    pub target: Target,
}

// --- Actions ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClickCmd {
    pub target: Target,
    pub double: bool,
    pub right: bool,
    pub middle: bool,
    pub force: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeCmd {
    pub target: Target,
    pub text: String,
    pub append: bool,
    pub enter: bool,
    pub delay: Option<f64>,
    pub clear: bool,
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClearCmd {
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PressCmd {
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeydownCmd {
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyupCmd {
    pub key: String, // "all" or specific key
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectCmd {
    pub target: Target,
    pub value: String, // Treating number as string for simplicity often works, or use specific types
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckCmd {
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UncheckCmd {
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HoverCmd {
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FocusCmd {
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrollCmd {
    pub direction: Option<String>, // up, down, left, right
    pub amount: Option<f64>,
    pub page: bool,
    pub timeout: Option<String>,
    pub target: Option<Target>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubmitCmd {
    pub target: Option<Target>,
}

// --- Wait ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WaitCondition {
    Load,
    Idle,
    Navigation,
    Ready,
    Visible(Target),
    Hidden(Target),
    Exists(String),
    Gone(String),
    Url(String),
    Until(String),
    Items { selector: String, count: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitCmd {
    pub condition: WaitCondition,
    pub timeout: Option<String>,
}

// --- Extract ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExtractWhat {
    Links,
    Images,
    Tables,
    Meta,
    Text,
    Css(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractCmd {
    pub what: ExtractWhat,
    pub selector: Option<String>,
    pub format: Option<String>,
}

// --- Session ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CookiesCmd {
    pub action: CookiesAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CookiesAction {
    List,
    Get(String),
    Set { name: String, value: String },
    Delete(String),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageCmd {
    pub action: StorageAction,
    pub local: bool,
    pub session: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StorageAction {
    List,
    Get(String),
    Set { name: String, value: String },
    Delete(String),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMgmtCmd {
    pub action: Option<SessionAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionAction {
    New { name: String, mode: Option<String> },
    Close(String),
    Switch(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateCmd {
    pub action: StateAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StateAction {
    Save {
        path: String,
        cookies_only: bool,
        domain: Option<String>,
        include_session: bool,
    },
    Load {
        path: String,
        merge: bool,
        cookies_only: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeadersCmd {
    pub action: Option<HeadersAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HeadersAction {
    Set {
        domain: Option<String>,
        json: String,
    }, // Argument might be (domain, string) or just string
    Clear(Option<String>),
    Show(String), // The `domain_name` case
}

// --- Tabs ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabActionCmd {
    pub action: TabAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TabAction {
    New(String),
    Switch(f64),
    Close(Option<f64>),
}

// --- Intent ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginCmd {
    pub user: String,
    pub pass: String,
    pub no_submit: bool,
    pub wait: Option<String>,
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchCmd {
    pub query: String,
    pub submit: Option<String>,
    pub wait: Option<String>,
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DismissCmd {
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrollUntilCmd {
    pub target: Target,
    pub amount: Option<f64>,
    pub page: bool,
    pub timeout: Option<String>,
}

// --- Packs ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackActionCmd {
    pub action: String, // load or unload
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentsCmd {
    pub session: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefineCmd {
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UndefineCmd {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportCmd {
    pub name: String,
    pub out: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunCmd {
    pub name: String,
    pub params: Vec<(String, String)>,
}

// --- Network ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterceptCmd {
    pub rule: InterceptRule,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InterceptRule {
    Clear(Option<String>),
    Set {
        pattern: String,
        block: bool,
        respond: Option<String>,
        respond_file: Option<String>,
        status: Option<f64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestsCmd {
    pub filter: Option<String>,
    pub method: Option<String>,
    pub last: Option<f64>,
}

// --- Console ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsoleCmd {
    pub clear: bool,
    pub level: Option<String>,
    pub filter: Option<String>,
    pub last: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorsCmd {
    pub clear: bool,
    pub last: Option<f64>,
}

// --- Frames ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameSwitchCmd {
    pub target: FrameTarget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FrameTarget {
    Main,
    Parent,
    Target(Target),
}

// --- Dialog ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DialogCmd {
    pub action: DialogAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DialogAction {
    Accept(Option<String>),
    Dismiss,
    Auto(String),
}

// --- Viewport ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewportSizeCmd {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceCmd {
    pub name: Option<String>, // None means reset
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaCmd {
    pub feature: Option<String>, // None means reset
    pub value: Option<String>,
}

// --- Recording ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceCmd {
    pub start: bool,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordCmd {
    pub start: bool,
    pub path: Option<String>,
    pub quality: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HighlightCmd {
    pub clear: bool,
    pub target: Option<Target>,
    pub duration: Option<String>,
    pub color: Option<String>,
}

// --- Utility ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfCmd {
    pub path: String,
    pub format: Option<String>,
    pub landscape: bool,
    pub margin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnCmd {
    pub action: String, // status, save, discard, show
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelpCmd {
    pub topic: Option<String>,
}

// --- Target ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub atomic: TargetAtomic,
    pub relation: Option<Box<TargetRelation>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetRelation {
    pub kind: RelationKind,
    pub target: Target,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RelationKind {
    Near,
    Inside,
    After,
    Before,
    Contains,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TargetAtomic {
    Id(usize),
    Text(String),
    Selector { kind: String, value: String }, // css or xpath
    Role(String),
}

// --- Conversion to/from oryn_common::resolver::Target ---

impl Target {
    /// Convert ast::Target to oryn_common::resolver::Target for resolution.
    pub fn to_resolver_target(&self) -> oryn_common::resolver::Target {
        use oryn_common::resolver::Target as ResolverTarget;

        let base = match &self.atomic {
            TargetAtomic::Id(id) => ResolverTarget::Id(*id),
            TargetAtomic::Text(text) => ResolverTarget::Text(text.clone()),
            TargetAtomic::Role(role) => ResolverTarget::Role(role.clone()),
            TargetAtomic::Selector { value, .. } => ResolverTarget::Selector(value.clone()),
        };

        let Some(relation) = &self.relation else {
            return base;
        };

        let base_box = Box::new(base);
        let related_box = Box::new(relation.target.to_resolver_target());

        match relation.kind {
            RelationKind::Near => ResolverTarget::Near {
                target: base_box,
                anchor: related_box,
            },
            RelationKind::Inside => ResolverTarget::Inside {
                target: base_box,
                container: related_box,
            },
            RelationKind::After => ResolverTarget::After {
                target: base_box,
                anchor: related_box,
            },
            RelationKind::Before => ResolverTarget::Before {
                target: base_box,
                anchor: related_box,
            },
            RelationKind::Contains => ResolverTarget::Contains {
                target: base_box,
                content: related_box,
            },
        }
    }

    /// Convert oryn_common::resolver::Target back to ast::Target after resolution.
    pub fn from_resolver_target(resolver_target: &oryn_common::resolver::Target) -> Self {
        use oryn_common::resolver::Target as ResolverTarget;

        fn simple(atomic: TargetAtomic) -> Target {
            Target {
                atomic,
                relation: None,
            }
        }

        fn with_relation(
            base: &ResolverTarget,
            related: &ResolverTarget,
            kind: RelationKind,
        ) -> Target {
            let base_target = Target::from_resolver_target(base);
            let related_target = Target::from_resolver_target(related);
            Target {
                atomic: base_target.atomic,
                relation: Some(Box::new(TargetRelation {
                    kind,
                    target: related_target,
                })),
            }
        }

        match resolver_target {
            ResolverTarget::Id(id) => simple(TargetAtomic::Id(*id)),
            ResolverTarget::Text(text) => simple(TargetAtomic::Text(text.clone())),
            ResolverTarget::Role(role) => simple(TargetAtomic::Role(role.clone())),
            ResolverTarget::Selector(sel) => simple(TargetAtomic::Selector {
                kind: "css".to_string(),
                value: sel.clone(),
            }),
            ResolverTarget::Infer => simple(TargetAtomic::Text(String::new())),
            ResolverTarget::Near { target, anchor } => {
                with_relation(target, anchor, RelationKind::Near)
            }
            ResolverTarget::Inside { target, container } => {
                with_relation(target, container, RelationKind::Inside)
            }
            ResolverTarget::After { target, anchor } => {
                with_relation(target, anchor, RelationKind::After)
            }
            ResolverTarget::Before { target, anchor } => {
                with_relation(target, anchor, RelationKind::Before)
            }
            ResolverTarget::Contains { target, content } => {
                with_relation(target, content, RelationKind::Contains)
            }
        }
    }
}
