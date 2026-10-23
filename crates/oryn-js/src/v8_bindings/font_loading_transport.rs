// Broker-neutral request/terminal transport. Exact receiver ownership is
// (binding instance, document generation); epoch is reserved and currently zero.
// Parser stylesheet producers carry their separate stream identity. A serial cannot
// authorize another realm or an earlier document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontLoadOwner {
    pub instance: u64,
    pub document: oryn_dom::NodeId,
    pub generation: u64,
    pub epoch: u64,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct FontDemandStamp {
    pub owner:FontLoadOwner,
    pub demand:u64,
}
pub struct FontSelection {
    pub indices:Vec<usize>,
    pub allocation:std::sync::Arc<dyn Send+Sync>,
}
pub struct FontRequestQueue {
    pub requests:std::collections::VecDeque<FontLoadRequest>,
    pub allocation:Option<std::sync::Arc<dyn Send+Sync>>,
}
pub enum FontSelectionError { Syntax, Resource(String) }
pub struct ConnectedFontFaces {
    pub faces:Vec<std::rc::Rc<dyn super::font_loading::NativeFontFace>>,
    pub allocation:Option<std::sync::Arc<dyn Send+Sync>>,
}
pub struct FontLayoutInputs {
    pub states:Vec<(oryn_dom::NodeId,Vec<String>)>,
    pub allocation:std::sync::Arc<dyn Send+Sync>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontLoadToken {
    pub owner: FontLoadOwner,
    pub face: u64,
    pub revision: u64,
    pub serial: u64,
}
pub struct ImplicitFontTokens {
    pub tokens:Vec<FontLoadToken>,
    pub allocation:Option<std::sync::Arc<dyn Send+Sync>>,
}
impl ImplicitFontTokens {
    pub fn empty()->Self{Self{tokens:Vec::new(),allocation:None}}
}
pub enum FontLoadSource {
    Url(String),
    Local(String),
    Bytes(Vec<u8>),
}
pub enum FontFaceSourceInput {
    Css { text:String, units:Vec<u16>, allocation:Option<std::sync::Arc<dyn Send+Sync>> },
    Binary { bytes:Vec<u8>, allocation:std::sync::Arc<dyn Send+Sync> },
}
// The output reservation follows the native bytes until the consuming V8
// string has been allocated. It must not be discarded by the native adapter.
pub struct FontDescriptorValue {
    pub value: String,
    pub allocation: std::sync::Arc<dyn Send + Sync>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FontStyleAxes {
    Normal,
    Italic,
    Oblique(f32, f32),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontAxes {
    pub weight: (f32, f32),
    pub width: (f32, f32),
    pub style: FontStyleAxes,
}
pub struct FontLoadRequest {
    pub token: FontLoadToken,
    pub family: String,
    pub descriptors: String,
    pub sources: Vec<FontLoadSource>,
    pub base_url: String,
    pub text: String,
    pub allocation: std::sync::Arc<dyn Send + Sync>,
    // The parsed native snapshot retains its original owned strings and
    // descriptor/condition allocations independently of transport copies.
    pub source_allocation: std::sync::Arc<dyn Send + Sync>,
}
#[derive(Clone,Copy)]
pub enum FontLoadTerminal {
    Decoded,
    Exhausted,
    Cancelled,
}
pub struct FontLoadCompletion {
    pub token: FontLoadToken,
    pub terminal: FontLoadTerminal,
}
