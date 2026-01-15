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
}

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

/// Responses received from the scanner.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScannerProtocolResponse {
    Ok {
        #[serde(flatten)]
        data: ScannerData,
        #[serde(default)]
        warnings: Vec<String>,
    },
    Error {
        code: String,
        message: String,
        #[serde(default)]
        details: Option<serde_json::Value>,
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
