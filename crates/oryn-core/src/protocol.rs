use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Requests sent to the scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ScannerRequest {
    Scan(ScanRequest),
    Click(ClickRequest),
    Type(TypeRequest),
    Scroll(ScrollRequest),
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
    /// Navigate to a URL (handled by background script, not content script)
    Navigate(NavigateRequest),
    /// Go back in browser history (handled by background script)
    Back(BackRequest),
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
    pub target: Option<String>,
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
    ScanValidation(ScanResponse),
    Scan(ScanResult),
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

// Keeping it separate for now in case we need to differentiate "response from scan command"
// vs "structure containing page/elements" used elsewhere.
pub type ScanResponse = ScanResult;

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

    #[serde(default)]
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
