use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

/// Custom deserializer for HashMap<String, String> that filters out null values.
/// This is needed because the scanner returns attributes with null values for missing attributes.
fn deserialize_nullable_string_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let map: HashMap<String, Option<String>> = HashMap::deserialize(deserializer)?;
    Ok(map
        .into_iter()
        .filter_map(|(k, v)| v.map(|val| (k, val)))
        .collect())
}

/// Unified Action enum wrapping all action types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Action {
    Scanner(ScannerAction),
    Browser(BrowserAction),
    Session(SessionAction),
    Meta(MetaAction),
}

/// Actions executed by the content script (Scanner).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ScannerAction {
    Scan(ScanRequest),
    Click(ClickRequest),
    Type(TypeRequest),
    Scroll(ScrollRequest),
    #[serde(rename = "wait_for")]
    Wait(WaitRequest),
    Check(CheckRequest),
    Select(SelectRequest),
    Submit(SubmitRequest),
    Hover(HoverRequest),
    Focus(FocusRequest),
    Clear(ClearRequest),
    Execute(ExecuteRequest),
    Extract(ExtractRequest),
    Login(LoginRequest),
    Search(SearchRequest),
    Dismiss(DismissRequest),
    Accept(AcceptRequest),
    #[serde(rename = "get_text")]
    GetText(GetTextRequest),
    #[serde(rename = "get_html")]
    GetHtml(GetHtmlRequest),
}

/// Actions executed by the browser automation driver (Puppeteer/Selenium equivalent).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserAction {
    Navigate(NavigateRequest),
    Back(BackRequest),
    Forward(ForwardRequest),
    Refresh(RefreshRequest),
    Screenshot(ScreenshotRequest),
    Pdf(PdfRequest),
    Tab(TabRequest),
    Frame(FrameRequest),
    Dialog(DialogRequest),
    Press(PressRequest),
}

/// Actions managed by the session manager (Cookies, Storage, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SessionAction {
    Cookie(CookieRequest),
    Storage(StorageRequest),
    Headers(HeadersRequest),
    Proxy(ProxyRequest),
}

/// Meta-actions for the Oryn runtime (Packs, Intents, Learning).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum MetaAction {
    Pack(PackRequest),
    Intent(IntentRequest),
    Learn(LearnRequest),
    Config(ConfigRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigateRequest {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanRequest {
    #[serde(default)]
    pub max_elements: Option<usize>,
    #[serde(default)]
    pub monitor_changes: bool,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub view_all: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near: Option<String>,
    #[serde(default)]
    pub viewport_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickRequest {
    pub id: u32,
    #[serde(default)]
    pub button: MouseButton,
    #[serde(default)]
    pub double: bool,
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    #[default]
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeRequest {
    pub id: u32,
    pub text: String,
    #[serde(default)]
    pub clear: bool,
    #[serde(default)]
    pub submit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollRequest {
    pub id: Option<u32>, // None = window
    pub direction: ScrollDirection,
    pub amount: Option<String>, // "page", "half", "100px"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitRequest {
    pub condition: String, // "visible", "hidden", "url", "title"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRequest {
    pub id: u32,
    pub state: bool, // true = check, false = uncheck
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectRequest {
    pub id: u32,
    pub value: Option<String>,
    pub index: Option<usize>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitRequest {
    pub id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoverRequest {
    pub id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusRequest {
    pub id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClearRequest {
    pub id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteRequest {
    pub script: String,
    pub args: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractRequest {
    pub source: String, // "links", "images", "tables", "meta", "css"
    pub selector: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DismissRequest {
    pub target: String, // "popups", "modals", "cookie_banners"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptRequest {
    pub target: String, // "cookies"
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GetTextRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GetHtmlRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default)]
    pub outer: bool, // true = outerHTML, false = innerHTML
}

/// Responses received from the scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScannerProtocolResponse {
    Ok {
        #[serde(flatten)]
        data: Box<ScannerData>,
        #[serde(default)]
        warnings: Vec<String>,
    },
    Error {
        code: String,
        message: String,
        #[serde(default)]
        details: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScannerData {
    Scan(Box<ScanResult>),
    Action(ActionResult),
    Value(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub page: PageInfo,
    pub elements: Vec<Element>,
    pub stats: ScanStats,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patterns: Option<DetectedPatterns>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<Vec<ElementChange>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_intents: Option<Vec<IntentAvailability>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentAvailability {
    pub name: String,
    pub status: AvailabilityStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityStatus {
    Ready,
    NavigateRequired,
    MissingPattern,
    Unavailable,
}

/// Detected UI patterns on the page.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DetectedPatterns {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<LoginPattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchPattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<PaginationPattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modal: Option<ModalPattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cookie_banner: Option<CookieBannerPattern>,
}

/// Login form pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginPattern {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<u32>,
    pub password: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remember: Option<u32>,
}

/// Search box pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchPattern {
    pub input: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit: Option<u32>,
}

/// Pagination pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationPattern {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<u32>,
    #[serde(default)]
    pub pages: Vec<u32>,
}

/// Modal/dialog pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModalPattern {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel: Option<u32>,
}

/// Cookie consent banner pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieBannerPattern {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<u32>,
}

/// Element change between scans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementChange {
    pub id: u32,
    pub change_type: ChangeType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_value: Option<String>,
}

/// Changes summary for an entire page/session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PageChanges {
    pub url: Option<String>,
    pub title: Option<String>,
    #[serde(default)]
    pub removed: Vec<String>, // Selectors/Descriptions
    #[serde(default)]
    pub added: Vec<String>,
}

/// Type of change detected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Appeared,
    Disappeared,
    TextChanged,
    StateChanged,
    PositionChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageInfo {
    pub url: String,
    pub title: String,
    pub viewport: ViewportInfo,
    pub scroll: ScrollInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ViewportInfo {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ScrollInfo {
    pub x: u32,
    pub y: u32,
    pub max_x: u32,
    pub max_y: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub id: u32,
    #[serde(rename = "type")]
    pub element_type: String, // "input", "button", "link", etc.
    pub role: Option<String>,
    pub text: Option<String>,
    pub label: Option<String>,
    pub value: Option<String>,
    pub placeholder: Option<String>,

    pub selector: String,
    pub xpath: Option<String>,

    pub rect: Rect,

    #[serde(default, deserialize_with = "deserialize_nullable_string_map")]
    pub attributes: HashMap<String, String>,
    #[serde(default)]
    pub state: ElementState,

    #[serde(default)]
    pub children: Vec<u32>, // IDs of children
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ElementState {
    pub checked: bool,
    pub selected: bool,
    pub disabled: bool,
    pub readonly: bool,
    pub expanded: bool,
    pub focused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanStats {
    pub total: usize,
    #[serde(default)]
    pub scanned: usize,
    // duration_ms moved to top-level timing in response
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    pub success: bool,
    pub message: Option<String>,
    pub navigation: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub id: String,
    pub url: String,
    pub title: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ForwardRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RefreshRequest {
    pub hard: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotRequest {
    pub output: Option<String>,
    pub format: Option<String>, // "png", "jpeg"
    pub selector: Option<String>,
    pub fullpage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfRequest {
    pub path: String,
    pub format: Option<String>, // "A4", "Letter"
    pub landscape: bool,
    pub margin: Option<String>,
    pub scale: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabRequest {
    pub action: String, // "new", "switch", "close", "list"
    pub url: Option<String>,
    pub tab_id: Option<String>, // String or u32? Using String for flexibility
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameRequest {
    pub action: String, // "switch"
    pub target: Option<String>, // "main", "parent", "iframe_selector"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogRequest {
    pub action: String, // "accept", "dismiss", "auto_accept", "auto_dismiss"
    pub prompt_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PressRequest {
    pub key: String, // Main key
    pub modifiers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieRequest {
    pub action: String, // "get", "set", "delete", "clear", "list"
    pub name: Option<String>,
    pub value: Option<String>,
    pub domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageRequest {
    pub action: String, // "get", "set", "delete", "clear", "list"
    pub storage_type: String, // "local", "session"
    pub key: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadersRequest {
    pub action: String, // "set", "clear", "view"
    pub headers: Option<std::collections::HashMap<String, String>>,
    pub domain: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyRequest {
    pub action: String, // "set", "clear"
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackRequest {
    pub action: String, // "load", "unload", "list"
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentRequest {
    pub action: String, // "define", "undefine", "run", "list", "export"
    pub name: Option<String>,
    pub params: Option<std::collections::HashMap<String, String>>,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnRequest {
    pub action: String, // "status", "save", "discard", "show"
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigRequest {
    pub key: String,
    pub value: String,
}
