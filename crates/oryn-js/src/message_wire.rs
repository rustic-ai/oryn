//! V8-free structured-message transport and context-owned endpoint authority.
//! Opaque endpoint tokens are capabilities: author JSON must never access this API.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_MESSAGES: usize = 1024;
pub const MAX_PORTS: usize = 4096;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortToken {
    pub id: u64,
    pub secret: [u8; 32],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireHost {
    Quad,
    Geometry {
        rect: bool,
        matrix: bool,
        readonly: bool,
        is_2d: bool,
        bits: Vec<u64>,
    },
    ImageData {
        width: u32,
        height: u32,
    },
    Port(u32),
    Blob {
        bytes: Vec<u8>,
        media_type: String,
        file: Option<WireFile>,
    },
    DomException {
        name: String,
        message: String,
        code: i32,
        #[serde(default)]
        stack: Vec<u16>,
    },
    // Appended within the pending native contract batch; prior variant indices stay stable.
    FileList,
    Stream {
        readable: Option<u32>,
        writable: Option<u32>,
    },
    CryptoKey(crate::crypto_key::Key),
    ImageBitmap {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    OffscreenCanvas {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        origin_clean: bool,
    },
    TransferredArrayBuffer {
        index: u32,
        bytes: Vec<u8>,
    },
    VideoFrame(VideoFrameData),
    AudioData(AudioDataRecord),
    MediaSourceHandle(u32),
    UserActivation {
        active: bool,
        sticky: bool,
    },
    ModuleOrigin([u8; 32]),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDataRecord {
    pub format: String,
    pub sample_rate: f32,
    pub frames: u32,
    pub channels: u32,
    pub timestamp: i64,
    pub bytes: Vec<u8>,
}
impl AudioDataRecord {
    pub fn sample_bytes(format: &str) -> Option<usize> {
        match format {
            "u8" | "u8-planar" => Some(1),
            "s16" | "s16-planar" => Some(2),
            "s32" | "s32-planar" | "f32" | "f32-planar" => Some(4),
            _ => None,
        }
    }
    pub fn expected_bytes(&self) -> Option<usize> {
        if self.frames == 0
            || self.channels == 0
            || !self.sample_rate.is_finite()
            || self.sample_rate <= 0.
        {
            return None;
        }
        (self.frames as usize)
            .checked_mul(self.channels as usize)?
            .checked_mul(Self::sample_bytes(&self.format)?)
    }
    pub fn valid(&self) -> bool {
        self.expected_bytes() == Some(self.bytes.len()) && self.bytes.len() <= MAX_BYTES
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFrameData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub planes: Option<Vec<u8>>,
    pub timestamp: i64,
    pub duration: Option<u64>,
    pub visible: [u32; 4],
    pub display: [u32; 2],
    pub rotation: u16,
    pub flip: bool,
}
impl VideoFrameData {
    pub fn i420_size(width: u32, height: u32) -> Option<usize> {
        if width == 0 || height == 0 {
            return None;
        }
        let size = (width as usize).checked_mul(height as usize)?.checked_add(
            (width as usize)
                .div_ceil(2)
                .checked_mul((height as usize).div_ceil(2))?
                .checked_mul(2)?,
        )?;
        (size <= MAX_BYTES).then_some(size)
    }
    fn valid(&self) -> bool {
        self.planes
            .as_ref()
            .is_none_or(|planes| Self::i420_size(self.width, self.height) == Some(planes.len()))
            && self.width > 0
            && self.height > 0
            && (self.width as usize)
                .checked_mul(self.height as usize)
                .and_then(|n| n.checked_mul(4))
                == Some(self.rgba.len())
            && self.visible[2] > 0
            && self.visible[3] > 0
            && self.visible[0]
                .checked_add(self.visible[2])
                .is_some_and(|end| end <= self.width)
            && self.visible[1]
                .checked_add(self.visible[3])
                .is_some_and(|end| end <= self.height)
            && self.display.iter().all(|value| *value > 0)
            && matches!(self.rotation, 0 | 90 | 180 | 270)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireFile {
    pub name: String,
    pub last_modified: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireValue {
    #[serde(default)]
    pub shared: Vec<crate::shared_memory::SharedMemoryReference>,
    #[serde(skip)]
    pub shared_queue: Option<std::sync::Arc<dyn crate::shared_memory::SharedMessageLease>>,
    #[serde(skip)]
    pub shared_recipient: Option<crate::shared_memory::SharedMemoryRecipient>,
    #[serde(default)]
    pub wasm_modules: Vec<Vec<u8>>,
    pub version: u32,
    pub bytes: Vec<u8>,
    pub hosts: Vec<WireHost>,
    pub buffers: Vec<Vec<u8>>,
    pub ports: Vec<PortToken>,
}
impl WireValue {
    pub fn user_activation(&self) -> Option<[bool; 2]> {
        self.hosts.iter().find_map(|host| {
            if let WireHost::UserActivation { active, sticky } = host {
                Some([*active, *sticky])
            } else {
                None
            }
        })
    }
    pub fn set_user_activation(&mut self, value: Option<[bool; 2]>) {
        if let Some([active, sticky]) = value {
            self.hosts.push(WireHost::UserActivation { active, sticky });
        }
    }
    /// Parent failure cleanup retains transfer claims but avoids copying body
    /// bytes. Both shared backing leases and ports must follow the task owner.
    pub fn cleanup_clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            shared_queue: self.shared_queue.clone(),
            shared_recipient: self.shared_recipient.clone(),
            wasm_modules: Vec::new(),
            version: 1,
            bytes: Vec::new(),
            hosts: Vec::new(),
            buffers: Vec::new(),
            ports: self.ports.clone(),
        }
    }
    pub fn retained_bytes(&self) -> usize {
        self.hosts
            .iter()
            .fold(
                self.wasm_modules
                    .iter()
                    .fold(self.bytes.len(), |size, module| {
                        size.saturating_add(module.len())
                    }),
                |n, h| {
                    n.saturating_add(match h {
                        WireHost::TransferredArrayBuffer { bytes, .. } => bytes.len(),
                        WireHost::CryptoKey(key) => key.retained_bytes(),
                        WireHost::Blob {
                            bytes,
                            media_type,
                            file,
                        } => bytes
                            .len()
                            .saturating_add(media_type.len())
                            .saturating_add(file.as_ref().map_or(0, |f| f.name.len())),
                        WireHost::AudioData(data) => data
                            .bytes
                            .len()
                            .saturating_add(data.format.len())
                            .saturating_add(32),
                        WireHost::VideoFrame(frame) => frame
                            .rgba
                            .len()
                            .saturating_add(frame.planes.as_ref().map_or(0, Vec::len))
                            .saturating_add(64),
                        WireHost::ImageBitmap { rgba, .. }
                        | WireHost::OffscreenCanvas { rgba, .. } => rgba.len().saturating_add(16),
                        WireHost::Geometry { bits, .. } => bits.len().saturating_mul(8),
                        WireHost::DomException {
                            name,
                            message,
                            stack,
                            ..
                        } => name
                            .len()
                            .saturating_add(message.len())
                            .saturating_add(stack.len().saturating_mul(2)),
                        _ => 32,
                    })
                },
            )
            .saturating_add(
                self.buffers
                    .iter()
                    .fold(0usize, |n, b| n.saturating_add(b.len())),
            )
            .saturating_add(self.ports.len().saturating_mul(64))
            .saturating_add(self.shared.len().saturating_mul(160))
    }
    pub fn validate(&self) -> Result<(), String> {
        let origins = self
            .hosts
            .iter()
            .filter(|host| matches!(host, WireHost::ModuleOrigin(_)))
            .count();
        if origins > 1 || (!self.wasm_modules.is_empty() && origins != 1) {
            return Err("Wasm module origin metadata missing or duplicate".into());
        }
        if self.version != 1
            || self.shared.len() > 1024
            || self.wasm_modules.len() > 1024
            || self.retained_bytes() > MAX_BYTES
            || self.hosts.len() > 100_000
            || self.buffers.len() > MAX_PORTS
            || self.ports.len() > MAX_PORTS
        {
            return Err("structured message limit or version".into());
        }
        let mut ids = BTreeSet::new();
        for port in &self.ports {
            if !ids.insert(port.id) {
                return Err("duplicate transferred port".into());
            }
        }
        if self
            .hosts
            .iter()
            .filter(|host| matches!(host, WireHost::UserActivation { .. }))
            .count()
            > 1
        {
            return Err("duplicate message activation snapshot".into());
        }
        let mut buffer_ids = BTreeSet::new();
        for host in &self.hosts {
            if let WireHost::TransferredArrayBuffer { index, bytes } = host
                && (bytes.is_empty()
                    || !self.buffers.get(*index as usize).is_some_and(Vec::is_empty)
                    || !buffer_ids.insert(*index))
            {
                return Err("invalid transferred buffer metadata".into());
            }
        }
        let mut stream_ports = BTreeSet::new();
        for host in &self.hosts {
            if let WireHost::Stream { readable, writable } = host {
                if readable.is_none() && writable.is_none() {
                    return Err("empty stream transfer".into());
                }
                for index in readable.iter().chain(writable.iter()) {
                    if *index as usize >= self.ports.len() || !stream_ports.insert(*index) {
                        return Err("invalid stream endpoint".into());
                    }
                }
            }
        }
        for host in &self.hosts {
            if let WireHost::MediaSourceHandle(index) = host
                && (*index as usize >= self.ports.len() || !stream_ports.insert(*index))
            {
                return Err("invalid media source endpoint".into());
            }
        }
        for host in &self.hosts {
            match host {
                WireHost::CryptoKey(key) => key
                    .validate()
                    .map_err(|_| "invalid CryptoKey clone".to_string())?,
                WireHost::Port(id)
                    if *id as usize >= self.ports.len() || stream_ports.contains(id) =>
                {
                    return Err("invalid host port index".into());
                }
                WireHost::AudioData(data) if !data.valid() => {
                    return Err("invalid audio data clone".into());
                }
                WireHost::VideoFrame(frame) if !frame.valid() => {
                    return Err("invalid video frame clone".into());
                }
                WireHost::OffscreenCanvas {
                    width,
                    height,
                    rgba,
                    ..
                } if (*width as usize)
                    .checked_mul(*height as usize)
                    .and_then(|size| size.checked_mul(4))
                    != Some(rgba.len()) =>
                {
                    return Err("invalid offscreen clone".into());
                }
                WireHost::ImageBitmap {
                    width,
                    height,
                    rgba,
                } if *width == 0
                    || *height == 0
                    || (*width as usize)
                        .checked_mul(*height as usize)
                        .and_then(|size| size.checked_mul(4))
                        != Some(rgba.len()) =>
                {
                    return Err("invalid bitmap clone".into());
                }
                WireHost::Geometry {
                    rect, matrix, bits, ..
                } if (*rect && *matrix) || bits.len() != if *matrix { 16 } else { 4 } => {
                    return Err("invalid geometry clone".into());
                }
                _ => {}
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TargetOrigin {
    Any,
    SameOrigin,
    Tuple(String),
    Never,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WindowTarget {
    Root,
    Frame(String),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowMessage {
    pub source_page: u64,
    pub source: WindowTarget,
    pub source_origin: String,
    pub payload: WireValue,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageRequest {
    InDocument {
        target: WindowTarget,
        document_generation: u64,
        stream_epoch: u64,
        request: Box<MessageRequest>,
    },
    RegisterRootDocument {
        document_generation: u64,
        stream_epoch: u64,
    },
    RetireRootDocument {
        document_generation: u64,
        stream_epoch: u64,
    },
    ParkDocument {
        document_generation: u64,
        stream_epoch: u64,
    },
    ParkFrameDocument {
        creator: crate::worker_wire::Creator,
    },
    RestoreFrameDocument {
        token: ParkedDocumentToken,
        parent: crate::worker_wire::Creator,
        frame: String,
    },
    DiscardFrameDocument {
        token: ParkedDocumentToken,
    },
    RestoreDocument {
        token: ParkedDocumentToken,
    },
    DiscardDocument {
        token: ParkedDocumentToken,
    },
    RegisterWindow {
        frame: String,
        document_generation: u64,
        stream_epoch: u64,
        origin: String,
        #[serde(default)]
        inherit_origin: Option<WindowTarget>,
    },
    RetireWindow {
        frame: String,
        document_generation: u64,
        stream_epoch: u64,
    },
    WindowStatus {
        page: u64,
        target: WindowTarget,
    },
    BroadcastOpen {
        target: WindowTarget,
        name: Vec<u16>,
    },
    BroadcastPost {
        channel: PortToken,
        payload: WireValue,
    },
    BroadcastPoll {
        channel: PortToken,
        max: usize,
    },
    BroadcastClose {
        channel: PortToken,
    },
    CreateChannel,
    Transfer {
        ports: Vec<PortToken>,
    },
    DiscardTransfers {
        ports: Vec<PortToken>,
    },
    PostPort {
        port: PortToken,
        payload: WireValue,
    },
    PollPort {
        port: PortToken,
        max: usize,
    },
    ClosePort {
        port: PortToken,
    },
    PostWindow {
        target_page: u64,
        target: WindowTarget,
        source: WindowTarget,
        target_origin: TargetOrigin,
        payload: WireValue,
    },
    PollWindow {
        target: WindowTarget,
        max: usize,
    },
    PortPending {
        port: PortToken,
    },
}
impl MessageRequest {
    /// Executable Window IPC must wrap these operations in native InDocument.
    /// Lifecycle registration and token-only cleanup have distinct admission.
    pub fn requires_window_document_scope(&self) -> bool {
        match self {
            Self::PortPending { .. }
            | Self::CreateChannel
            | Self::Transfer { .. }
            | Self::PostPort { .. }
            | Self::PollPort { .. }
            | Self::PostWindow { .. }
            | Self::PollWindow { .. }
            | Self::BroadcastOpen { .. }
            | Self::BroadcastPost { .. }
            | Self::BroadcastPoll { .. } => true,
            Self::InDocument { .. }
            | Self::RegisterRootDocument { .. }
            | Self::RetireRootDocument { .. }
            | Self::RegisterWindow { .. }
            | Self::RetireWindow { .. }
            | Self::ParkFrameDocument { .. }
            | Self::RestoreFrameDocument { .. }
            | Self::DiscardFrameDocument { .. }
            | Self::ParkDocument { .. }
            | Self::RestoreDocument { .. }
            | Self::DiscardDocument { .. }
            | Self::WindowStatus { .. }
            | Self::ClosePort { .. }
            | Self::DiscardTransfers { .. }
            | Self::BroadcastClose { .. } => false,
        }
    }
    pub fn wakes_recipients(&self) -> bool {
        match self {
            Self::InDocument { request, .. } => request.wakes_recipients(),
            Self::PostPort { .. }
            | Self::PostWindow { .. }
            | Self::ClosePort { .. }
            | Self::BroadcastPost { .. } => true,
            _ => false,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageResponse {
    DocumentParked(ParkedDocumentToken),
    DocumentRestored {
        document_generation: u64,
        stream_epoch: u64,
    },
    BroadcastOpened(PortToken),
    BroadcastMessages(Vec<BroadcastMessage>),
    WindowLive(bool),
    Channel([PortToken; 2]),
    Transferred(Vec<PortToken>),
    PortMessages {
        messages: Vec<WireValue>,
        peer_closed: bool,
    },
    WindowMessages(Vec<WindowMessage>),
    Done,
    PortPending(bool),
    CachedCompletionExpired,
}
pub trait MessageTransport: Send + Sync {
    fn request(&self, request: MessageRequest) -> Result<MessageResponse, String>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Principal {
    Window(u64),
    Agent(u64),
}
impl Principal {
    fn window(self) -> Result<u64, String> {
        match self {
            Self::Window(page) => Ok(page),
            Self::Agent(_) => Err("agent cannot act as a Window".into()),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastMessage {
    pub origin: String,
    pub payload: WireValue,
}
struct BroadcastSubscription {
    document: Option<crate::shared_memory::SharedMemoryRecipient>,
    secret: [u8; 32],
    owner: Principal,
    partition: String,
    origin_key: String,
    origin: String,
    name: Vec<u16>,
    queue: VecDeque<(u64, BroadcastMessage)>,
    bytes: usize,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Bound(Principal),
    Transit(Principal),
    Queued,
    Bridge,
}
struct Endpoint {
    secret: [u8; 32],
    owner: Owner,
    peer: Option<u64>,
    closed: bool,
    close_pending: bool,
    queue: VecDeque<WireValue>,
    bytes: usize,
}
struct QueuedWindow {
    message: WindowMessage,
    required_origin: Option<String>,
}
struct WindowRegistration {
    document_generation: u64,
    stream_epoch: u64,
    origin: String,
    origin_key: String,
}
/// One-use native history capability. Never expose through an author binding.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ParkedDocumentToken([u8; 32]);
#[path = "message_prerender.rs"]
mod message_prerender;
use message_prerender::PrerenderDocument;
pub use message_prerender::{
    PrerenderFrameOrigin, PrerenderFrameRegistration, PrerenderMessage, PrerenderToken,
};
impl std::fmt::Debug for ParkedDocumentToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ParkedDocumentToken([redacted])")
    }
}
struct ParkedDocument {
    page: u64,
    frame: Option<String>,
    lifetime: (u64, u64),
    frames: BTreeMap<String, WindowRegistration>,
    queues: BTreeMap<WindowTarget, VecDeque<QueuedWindow>>,
    bytes: BTreeMap<WindowTarget, usize>,
}
type PendingSharedDisposals = BTreeMap<
    (Principal, crate::shared_memory::SharedMemoryHandoff),
    (
        Option<std::sync::Arc<dyn crate::shared_memory::SharedMessageBackend>>,
        crate::shared_memory::SharedMemoryReference,
    ),
>;

#[derive(Default)]
pub struct MessageHub {
    prerender_token_key: Option<[u8; 32]>,
    prerender_token_sequence: u64,
    prerender_issued: BTreeMap<u64, (u64, u64)>,
    prerenders: BTreeMap<[u8; 32], PrerenderDocument>,
    park_token_key: Option<[u8; 32]>,
    park_token_sequence: u64,
    broadcast_sequence: u64,
    pending_shared_disposals: PendingSharedDisposals,
    shared_backend: Option<std::sync::Arc<dyn crate::shared_memory::SharedMessageBackend>>,
    parked: BTreeMap<[u8; 32], ParkedDocument>,
    parked_frame_lifetimes: BTreeMap<(u64, String), (u64, u64)>,
    retired_roots: BTreeMap<u64, (u64, u64)>,
    roots: BTreeMap<u64, (u64, u64)>,
    next: u64,
    ports: BTreeMap<u64, Endpoint>,
    windows: BTreeMap<(u64, WindowTarget), VecDeque<QueuedWindow>>,
    window_bytes: BTreeMap<(u64, WindowTarget), usize>,
    frames: BTreeMap<(u64, String), WindowRegistration>,
    broadcasts: BTreeMap<u64, BroadcastSubscription>,
    partitions: BTreeMap<Principal, String>,
}
fn secret() -> Result<[u8; 32], String> {
    let mut value = [0; 32];
    getrandom::fill(&mut value).map_err(|_| "message capability entropy unavailable")?;
    Ok(value)
}
// Dedicated per-domain keys prevent cache and prerender capabilities from
// authenticating in one another's registries. The full AES block is retained.
fn mint_document_capability(
    key: &mut Option<[u8; 32]>,
    issued: &mut u64,
    page: u64,
) -> Result<[u8; 32], String> {
    use aes::cipher::{BlockEncrypt, KeyInit};
    let sequence = issued
        .checked_add(1)
        .ok_or("document capability sequence exhausted")?;
    if key.is_none() {
        *key = Some(secret()?);
    }
    let mut token = [0; 32];
    token[..8].copy_from_slice(&page.to_be_bytes());
    token[8..16].copy_from_slice(&sequence.to_be_bytes());
    let cipher = aes::Aes256::new_from_slice(key.as_ref().unwrap())
        .map_err(|_| "document capability key invalid")?;
    let mut tag = aes::cipher::Block::<aes::Aes256>::clone_from_slice(&token[..16]);
    cipher.encrypt_block(&mut tag);
    token[16..].copy_from_slice(&tag);
    *issued = sequence;
    Ok(token)
}
fn authentic_document_capability(
    key: Option<&[u8; 32]>,
    issued: u64,
    page: u64,
    token: &[u8; 32],
) -> bool {
    use aes::cipher::{BlockEncrypt, KeyInit};
    let Some(key) = key else { return false };
    let Ok(cipher) = aes::Aes256::new_from_slice(key) else {
        return false;
    };
    let mut tag = aes::cipher::Block::<aes::Aes256>::clone_from_slice(&token[..16]);
    cipher.encrypt_block(&mut tag);
    let mut difference = 0u8;
    for (index, expected) in tag.iter().enumerate() {
        difference |= expected ^ token[16 + index];
    }
    for (index, expected) in page.to_be_bytes().iter().enumerate() {
        difference |= expected ^ token[index];
    }
    let sequence = u64::from_be_bytes(token[8..16].try_into().unwrap());
    difference == 0 && sequence != 0 && sequence <= issued
}
impl MessageHub {
    fn mint_park_token(&mut self, page: u64) -> Result<[u8; 32], String> {
        mint_document_capability(
            &mut self.park_token_key,
            &mut self.park_token_sequence,
            page,
        )
    }
    fn authentic_park_token(&self, page: u64, token: &ParkedDocumentToken) -> bool {
        authentic_document_capability(
            self.park_token_key.as_ref(),
            self.park_token_sequence,
            page,
            &token.0,
        )
    }
    pub fn cached_token_expired(&self, page: u64, token: &ParkedDocumentToken) -> bool {
        self.authentic_park_token(page, token) && !self.parked.contains_key(&token.0)
    }
    pub fn set_shared_message_backend(
        &mut self,
        backend: std::sync::Arc<dyn crate::shared_memory::SharedMessageBackend>,
    ) {
        self.shared_backend = Some(backend);
    }
    /// Called only after native pagehide and before the isolate is suspended.
    /// The parent must hold live-page admission. No author-supplied inventory is
    /// accepted: frame origin identities and pending queues move from this hub.
    pub fn park_document_authority(
        &mut self,
        page: u64,
        generation: u64,
        epoch: u64,
    ) -> Result<ParkedDocumentToken, String> {
        self.root_document_authority(page, generation, epoch)?;
        if self.parked.len() >= MAX_PORTS {
            return Err("cached document authority capacity".into());
        }
        let token = self.mint_park_token(page)?;
        if self.parked.contains_key(&token) {
            return Err("cached document token collision".into());
        }
        let frame_keys = self
            .frames
            .keys()
            .filter(|(owner, _)| *owner == page)
            .cloned()
            .collect::<Vec<_>>();
        let queue_keys = self
            .windows
            .keys()
            .filter(|(owner, _)| *owner == page)
            .cloned()
            .collect::<Vec<_>>();
        let mut frames = BTreeMap::new();
        let mut queues = BTreeMap::new();
        let mut bytes = BTreeMap::new();
        for key in frame_keys {
            frames.insert(key.1.clone(), self.frames.remove(&key).unwrap());
        }
        for key in queue_keys {
            queues.insert(key.1.clone(), self.windows.remove(&key).unwrap());
            bytes.insert(key.1.clone(), self.window_bytes.remove(&key).unwrap_or(0));
        }
        self.roots.remove(&page);
        let lifetime = (generation, epoch);
        let high = self.retired_roots.entry(page).or_insert(lifetime);
        *high = (*high).max(lifetime);
        self.parked.insert(
            token,
            ParkedDocument {
                page,
                frame: None,
                lifetime,
                frames,
                queues,
                bytes,
            },
        );
        Ok(ParkedDocumentToken(token))
    }

    /// The parent supplies an authenticated native subtree, never a child inventory.
    pub fn park_frame_authority(
        &mut self,
        page: u64,
        frame: &str,
        generation: u64,
        epoch: u64,
        subtree: &[(String, u64, u64)],
    ) -> Result<ParkedDocumentToken, String> {
        self.frame_document_authority(page, frame, generation, epoch)?;
        if self.parked.len() >= MAX_PORTS
            || self
                .parked_frame_lifetimes
                .len()
                .saturating_add(subtree.len())
                > MAX_PORTS
        {
            return Err("cached frame authority capacity".into());
        }
        let mut selected = std::collections::BTreeSet::new();
        for (frame, generation, epoch) in subtree {
            self.frame_document_authority(page, frame, *generation, *epoch)?;
            if !selected.insert(frame.clone()) {
                return Err("duplicate cached frame".into());
            }
        }
        if !selected.contains(frame) {
            return Err("cached subtree omits root frame".into());
        }
        let token = self.mint_park_token(page)?;
        if self.parked.contains_key(&token) {
            return Err("cached frame token collision".into());
        }
        let mut frames = BTreeMap::new();
        let mut queues = BTreeMap::new();
        let mut bytes = BTreeMap::new();
        for frame in selected {
            let registration = self.frames.remove(&(page, frame.clone())).unwrap();
            let high = self
                .parked_frame_lifetimes
                .entry((page, frame.clone()))
                .or_insert((registration.document_generation, registration.stream_epoch));
            *high = (*high).max((registration.document_generation, registration.stream_epoch));
            frames.insert(frame.clone(), registration);
            let target = WindowTarget::Frame(frame);
            if let Some(queue) = self.windows.remove(&(page, target.clone())) {
                queues.insert(target.clone(), queue);
            }
            if let Some(size) = self.window_bytes.remove(&(page, target.clone())) {
                bytes.insert(target, size);
            }
        }
        self.parked.insert(
            token,
            ParkedDocument {
                page,
                frame: Some(frame.into()),
                lifetime: (generation, epoch),
                frames,
                queues,
                bytes,
            },
        );
        Ok(ParkedDocumentToken(token))
    }
    pub fn restore_frame_authority(
        &mut self,
        page: u64,
        frame: &str,
        token: &ParkedDocumentToken,
    ) -> Result<(u64, u64), String> {
        let document = self
            .parked
            .get(&token.0)
            .ok_or("cached frame capability retired")?;
        if document.page != page || document.frame.as_deref() != Some(frame) {
            return Err("cached frame capability target mismatch".into());
        }
        if !self.roots.contains_key(&page) {
            return Err("cached frame root is inactive".into());
        }
        if document.frames.keys().any(|frame| {
            self.frames.contains_key(&(page, frame.clone()))
                || self
                    .windows
                    .get(&(page, WindowTarget::Frame(frame.clone())))
                    .is_some_and(|queue| !queue.is_empty())
        }) {
            return Err("incumbent frame must be parked before restoration".into());
        }
        if self.frames.len().saturating_add(document.frames.len()) > MAX_PORTS {
            return Err("frame registration capacity".into());
        }
        let document = self.parked.remove(&token.0).unwrap();
        let lifetime = document.lifetime;
        for (frame, registration) in document.frames {
            self.frames.insert((page, frame), registration);
        }
        for (target, queue) in document.queues {
            self.windows.insert((page, target), queue);
        }
        for (target, bytes) in document.bytes {
            self.window_bytes.insert((page, target), bytes);
        }
        Ok(lifetime)
    }

    /// Parent history restoration only, after the incumbent is parked and the
    /// restored security origin is committed. Rejection leaves the token live.
    pub fn restore_document_authority(
        &mut self,
        page: u64,
        token: &ParkedDocumentToken,
    ) -> Result<(u64, u64), String> {
        let document = self
            .parked
            .get(&token.0)
            .ok_or("cached document capability is retired")?;
        if document.page != page {
            return Err("cached document capability belongs to another page".into());
        }
        if document.frame.is_some() {
            return Err("frame capability cannot restore root authority".into());
        }
        if self.roots.contains_key(&page)
            || self.frames.keys().any(|(owner, _)| *owner == page)
            || self
                .windows
                .iter()
                .any(|((owner, _), queue)| *owner == page && !queue.is_empty())
        {
            return Err("incumbent document authority must be parked before restoration".into());
        }
        if self.frames.len().saturating_add(document.frames.len()) > MAX_PORTS {
            return Err("frame registration capacity".into());
        }
        let document = self.parked.remove(&token.0).unwrap();
        let lifetime = document.lifetime;
        self.roots.insert(page, lifetime);
        for (frame, registration) in document.frames {
            self.frames.insert((page, frame), registration);
        }
        for (target, queue) in document.queues {
            self.windows.insert((page, target), queue);
        }
        for (target, bytes) in document.bytes {
            self.window_bytes.insert((page, target), bytes);
        }
        Ok(lifetime)
    }

    /// Cache eviction releases queued transferred ports without reviving any
    /// document. Repeated eviction is harmless; foreign ownership is rejected.
    pub fn discard_parked_document(
        &mut self,
        page: u64,
        token: &ParkedDocumentToken,
    ) -> Result<(), String> {
        let Some(document) = self.parked.get(&token.0) else {
            return if self.authentic_park_token(page, token) {
                Ok(())
            } else {
                Err("unknown or foreign cached capability".into())
            };
        };
        if document.page != page {
            return Err("cached document capability belongs to another page".into());
        }
        let document = self.parked.remove(&token.0).unwrap();
        if document.frame.is_none() {
            self.retire_broadcast_document(
                page,
                WindowTarget::Root,
                document.lifetime.0,
                document.lifetime.1,
            );
        }
        for (frame, registration) in &document.frames {
            self.retire_broadcast_document(
                page,
                WindowTarget::Frame(frame.clone()),
                registration.document_generation,
                registration.stream_epoch,
            );
        }
        for queue in document.queues.into_values() {
            for item in queue {
                self.discard(item.message.payload);
            }
        }
        Ok(())
    }

    fn window_origin<'a>(
        &'a self,
        page: u64,
        target: &WindowTarget,
        live: &'a BTreeMap<u64, String>,
    ) -> Option<&'a str> {
        let root = live.get(&page)?;
        match target {
            WindowTarget::Root => Some(root.as_str()),
            WindowTarget::Frame(frame) => self
                .frames
                .get(&(page, frame.clone()))
                .map(|entry| entry.origin.as_str()),
        }
    }
    /// Parent-only lookup; callers supply the authenticated live-page snapshot.
    pub fn window_security_origin(
        &self,
        page: u64,
        target: &WindowTarget,
        live: &BTreeMap<u64, String>,
        keys: &BTreeMap<u64, String>,
    ) -> Option<(String, String)> {
        Some((
            self.window_origin(page, target, live)?.to_string(),
            self.window_origin_key(page, target, live, keys)?,
        ))
    }
    fn window_origin_key(
        &self,
        page: u64,
        target: &WindowTarget,
        live: &BTreeMap<u64, String>,
        keys: &BTreeMap<u64, String>,
    ) -> Option<String> {
        self.window_origin(page, target, live)?;
        match target {
            WindowTarget::Root => keys.get(&page).cloned(),
            WindowTarget::Frame(frame) => self
                .frames
                .get(&(page, frame.clone()))
                .map(|entry| entry.origin_key.clone()),
        }
    }
    pub fn set_partition(&mut self, principal: Principal, partition: String) {
        self.partitions.insert(principal, partition);
    }
    pub fn retained_bytes(&self) -> usize {
        self.parked
            .values()
            .flat_map(|document| document.bytes.values())
            .fold(0usize, |sum, bytes| sum.saturating_add(*bytes))
            .saturating_add(
                self.ports
                    .values()
                    .fold(0usize, |sum, item| sum.saturating_add(item.bytes)),
            )
            .saturating_add(
                self.window_bytes
                    .values()
                    .fold(0usize, |sum, bytes| sum.saturating_add(*bytes)),
            )
            .saturating_add(
                self.broadcasts
                    .values()
                    .fold(0usize, |sum, item| sum.saturating_add(item.bytes)),
            )
    }
    fn queue_budget(&self, bytes: usize) -> Result<(), String> {
        if bytes > MAX_BYTES.saturating_sub(self.retained_bytes()) {
            Err("context message queue capacity".into())
        } else {
            Ok(())
        }
    }
    fn broadcast_document_active(
        &self,
        document: Option<&crate::shared_memory::SharedMemoryRecipient>,
    ) -> bool {
        match document {
            Some(crate::shared_memory::SharedMemoryRecipient::Window {
                page,
                target,
                document_generation,
                stream_epoch,
            }) => match target {
                WindowTarget::Root => {
                    self.roots.get(page) == Some(&(*document_generation, *stream_epoch))
                }
                WindowTarget::Frame(frame) => {
                    self.frames
                        .get(&(*page, frame.clone()))
                        .is_some_and(|entry| {
                            (entry.document_generation, entry.stream_epoch)
                                == (*document_generation, *stream_epoch)
                        })
                }
            },
            _ => true,
        }
    }
    fn retire_active_frame_broadcasts(&mut self, page: u64) {
        let frames = self
            .frames
            .iter()
            .filter(|((owner, _), _)| *owner == page)
            .map(|((_, frame), entry)| {
                (frame.clone(), entry.document_generation, entry.stream_epoch)
            })
            .collect::<Vec<_>>();
        for (frame, generation, epoch) in frames {
            self.retire_broadcast_document(page, WindowTarget::Frame(frame), generation, epoch);
        }
    }
    fn retire_broadcast_document(
        &mut self,
        page: u64,
        target: WindowTarget,
        generation: u64,
        epoch: u64,
    ) {
        let retired = crate::shared_memory::SharedMemoryRecipient::Window {
            page,
            target,
            document_generation: generation,
            stream_epoch: epoch,
        };
        self.broadcasts
            .retain(|_, entry| entry.document.as_ref() != Some(&retired));
    }
    fn broadcast(
        &self,
        caller: Principal,
        token: &PortToken,
    ) -> Result<&BroadcastSubscription, String> {
        let item = self
            .broadcasts
            .get(&token.id)
            .ok_or("InvalidStateError: BroadcastChannel is closed")?;
        if item.owner != caller || item.secret != token.secret {
            return Err("BroadcastChannel capability is foreign".into());
        }
        Ok(item)
    }
    fn endpoint(
        &self,
        caller: Principal,
        token: &PortToken,
        transit: bool,
    ) -> Result<&Endpoint, String> {
        let endpoint = self.ports.get(&token.id).ok_or("unknown MessagePort")?;
        if endpoint.closed
            || endpoint.secret != token.secret
            || endpoint.owner
                != if transit {
                    Owner::Transit(caller)
                } else {
                    Owner::Bound(caller)
                }
        {
            return Err("MessagePort authority is retired or foreign".into());
        }
        Ok(endpoint)
    }
    fn payload_authority(&self, caller: Principal, payload: &mut WireValue) -> Result<(), String> {
        payload.validate()?;
        for token in &payload.ports {
            self.endpoint(caller, token, true)?;
        }
        if !payload.shared.is_empty() && payload.shared_queue.is_none() {
            payload.shared_queue = Some(
                self.shared_backend
                    .as_ref()
                    .ok_or("shared message backend unavailable")?
                    .adopt(caller, &payload.shared)?,
            );
        }
        Ok(())
    }
    fn receive(
        &mut self,
        owner: Principal,
        payload: &mut WireValue,
        document: Option<&crate::shared_memory::SharedMemoryRecipient>,
    ) -> Result<(), String> {
        // Rotate all capabilities before publishing any to the destination.
        let secrets = payload
            .ports
            .iter()
            .map(|_| secret())
            .collect::<Result<Vec<_>, _>>()?;
        for token in &payload.ports {
            let p = self
                .ports
                .get(&token.id)
                .ok_or("missing transferred endpoint")?;
            if p.closed || p.secret != token.secret || p.owner != Owner::Queued {
                return Err("transferred endpoint already consumed".into());
            }
        }
        if let Some(lease) = &payload.shared_queue {
            if let Some(previous) = &payload.shared_recipient {
                self.shared_backend
                    .as_ref()
                    .ok_or("shared message backend unavailable")?
                    .discard_delivered(previous.principal(), &payload.shared)?;
            }
            let recipient = match owner {
                Principal::Agent(id) => crate::shared_memory::SharedMemoryRecipient::Agent(id),
                Principal::Window(_) => document
                    .cloned()
                    .ok_or("shared memory window delivery requires exact document authority")?,
            };
            payload.shared = lease.deliver(recipient.clone())?;
            payload.shared_recipient = Some(recipient);
        }
        for (token, secret) in payload.ports.iter_mut().zip(secrets) {
            let endpoint = self.ports.get_mut(&token.id).unwrap();
            endpoint.owner = Owner::Bound(owner);
            endpoint.secret = secret;
            token.secret = secret;
        }
        Ok(())
    }
    fn enqueue_transfers(&mut self, payload: &WireValue) {
        for token in &payload.ports {
            self.ports.get_mut(&token.id).unwrap().owner = Owner::Queued;
        }
    }
    fn discard(&mut self, payload: WireValue) {
        if let Some(owner) = payload.shared_recipient {
            let _ = self.dispose_shared_deliveries(owner.principal(), &payload.shared);
        }
        for port in payload.ports {
            self.close_id(port.id)
        }
    }
    fn close_id(&mut self, id: u64) {
        let mut pending = vec![id];
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(mut endpoint) = self.ports.remove(&id) else {
                continue;
            };
            endpoint.closed = true;
            endpoint.bytes = 0;
            pending.extend(
                endpoint
                    .queue
                    .drain(..)
                    .flat_map(|value| value.ports.into_iter().map(|p| p.id)),
            );
            let peer = endpoint.peer.take();
            if let Some(peer) = peer.and_then(|id| self.ports.get_mut(&id)) {
                peer.peer = None;
                peer.close_pending = true;
            }
        }
    }
    /// `live_windows` comes from the parent's live registry at request time.
    /// Poll performs target-origin matching against the current destination URL.
    pub fn handle(
        &mut self,
        caller: u64,
        request: MessageRequest,
        live_windows: &BTreeMap<u64, String>,
    ) -> Result<MessageResponse, String> {
        let keys = live_windows
            .iter()
            .map(|(page, origin)| {
                (
                    *page,
                    if origin == "null" {
                        format!("opaque-page:{page}")
                    } else {
                        format!("oryn:origin:{origin}")
                    },
                )
            })
            .collect();
        self.handle_with_origin_keys(caller, request, live_windows, &keys)
    }
    pub fn handle_with_origin_keys(
        &mut self,
        caller: u64,
        request: MessageRequest,
        live_windows: &BTreeMap<u64, String>,
        origin_keys: &BTreeMap<u64, String>,
    ) -> Result<MessageResponse, String> {
        if !live_windows.contains_key(&caller) {
            return Err("message source page is retired".into());
        }
        self.handle_bound(
            Principal::Window(caller),
            request,
            live_windows,
            origin_keys,
            None,
            None,
        )
    }
    /// Caller liveness, partition and origin are attested by AgentRegistry before
    /// invoking this method; no synthetic browsing-context/page record is used.
    pub fn handle_agent(
        &mut self,
        agent: u64,
        request: MessageRequest,
        serialized_origin: &str,
        authority: &str,
    ) -> Result<MessageResponse, String> {
        if matches!(
            request,
            MessageRequest::ParkFrameDocument { .. }
                | MessageRequest::RestoreFrameDocument { .. }
                | MessageRequest::DiscardFrameDocument { .. }
                | MessageRequest::ParkDocument { .. }
                | MessageRequest::RestoreDocument { .. }
                | MessageRequest::DiscardDocument { .. }
                | MessageRequest::RegisterRootDocument { .. }
                | MessageRequest::RetireRootDocument { .. }
                | MessageRequest::RegisterWindow { .. }
                | MessageRequest::RetireWindow { .. }
                | MessageRequest::WindowStatus { .. }
                | MessageRequest::PostWindow { .. }
                | MessageRequest::PollWindow { .. }
        ) {
            return Err("agent cannot invoke Window routing".into());
        }
        self.handle_bound(
            Principal::Agent(agent),
            request,
            &BTreeMap::new(),
            &BTreeMap::new(),
            Some((serialized_origin, authority)),
            None,
        )
    }
    /// Native worker creation uses the registered document's authority, never
    /// an origin supplied in the Worker constructor or its serialized options.
    /// The parent must separately hold live-page admission while calling this.
    pub fn frame_document_authority(
        &self,
        page: u64,
        frame: &str,
        generation: u64,
        epoch: u64,
    ) -> Result<(String, String), String> {
        let document = self
            .frames
            .get(&(page, frame.into()))
            .ok_or("worker creator frame is retired")?;
        if (document.document_generation, document.stream_epoch) != (generation, epoch) {
            return Err("worker creator document lifetime is stale".into());
        }
        Ok((document.origin.clone(), document.origin_key.clone()))
    }
    /// Native policy commit replaces renderer metadata before privileged admission.
    pub fn commit_frame_origin(
        &mut self,
        page: u64,
        frame: &str,
        generation: u64,
        epoch: u64,
        origin: (String, String),
    ) -> Result<(), String> {
        self.frame_document_authority(page, frame, generation, epoch)?;
        let document = self
            .frames
            .get_mut(&(page, frame.into()))
            .ok_or("frame retired during origin commit")?;
        document.origin = origin.0;
        document.origin_key = origin.1;
        Ok(())
    }
    /// Native parent inventory; callers still validate live page and policy.
    pub fn registered_documents(&self) -> Vec<(u64, WindowTarget, u64, u64)> {
        self.roots
            .iter()
            .map(|(page, (generation, epoch))| (*page, WindowTarget::Root, *generation, *epoch))
            .chain(self.frames.iter().map(|((page, frame), document)| {
                (
                    *page,
                    WindowTarget::Frame(frame.clone()),
                    document.document_generation,
                    document.stream_epoch,
                )
            }))
            .collect()
    }
    /// Native delivery/retention authority only. Parked documents remain
    /// ineligible for author operations and WindowStatus discovery.
    pub fn retained_document_authority(
        &self,
        page: u64,
        target: &WindowTarget,
        generation: u64,
        epoch: u64,
    ) -> Result<(), String> {
        let active = match target {
            WindowTarget::Root => self.root_document_authority(page, generation, epoch),
            WindowTarget::Frame(frame) => self
                .frame_document_authority(page, frame, generation, epoch)
                .map(|_| ()),
        };
        if active.is_ok() {
            return Ok(());
        }
        if self.parked.values().any(|document| {
            document.page == page
                && match target {
                    WindowTarget::Root => document.lifetime == (generation, epoch),
                    WindowTarget::Frame(frame) => document.frames.get(frame).is_some_and(|entry| {
                        (entry.document_generation, entry.stream_epoch) == (generation, epoch)
                    }),
                }
        }) {
            Ok(())
        } else {
            Err("retained document lifetime is retired".into())
        }
    }
    pub fn root_document_identity(&self, page: u64) -> Option<(u64, u64)> {
        self.roots.get(&page).copied()
    }
    pub fn root_document_authority(
        &self,
        page: u64,
        generation: u64,
        epoch: u64,
    ) -> Result<(), String> {
        if self.roots.get(&page) != Some(&(generation, epoch)) {
            return Err("worker creator root document lifetime is stale or retired".into());
        }
        Ok(())
    }
    fn handle_bound(
        &mut self,
        caller: Principal,
        request: MessageRequest,
        live_windows: &BTreeMap<u64, String>,
        origin_keys: &BTreeMap<u64, String>,
        agent_origin: Option<(&str, &str)>,
        document: Option<crate::shared_memory::SharedMemoryRecipient>,
    ) -> Result<MessageResponse, String> {
        match request {
            MessageRequest::InDocument {
                target,
                document_generation,
                stream_epoch,
                request,
            } => {
                let page = caller.window()?;
                match &target {
                    WindowTarget::Root => {
                        self.root_document_authority(page, document_generation, stream_epoch)?
                    }
                    WindowTarget::Frame(frame) => {
                        self.frame_document_authority(
                            page,
                            frame,
                            document_generation,
                            stream_epoch,
                        )?;
                    }
                }
                if !matches!(
                    request.as_ref(),
                    MessageRequest::PortPending { .. }
                        | MessageRequest::PollPort { .. }
                        | MessageRequest::BroadcastPoll { .. }
                        | MessageRequest::PollWindow { .. }
                        | MessageRequest::WindowStatus { .. }
                        | MessageRequest::CreateChannel
                        | MessageRequest::Transfer { .. }
                        | MessageRequest::PostPort { .. }
                        | MessageRequest::PostWindow { .. }
                        | MessageRequest::BroadcastOpen { .. }
                        | MessageRequest::BroadcastPost { .. }
                ) {
                    return Err("invalid scoped message operation".into());
                }
                if let MessageRequest::PostWindow { source, .. } = request.as_ref()
                    && source != &target
                {
                    return Err("scoped Window message source mismatch".into());
                }
                if let MessageRequest::BroadcastOpen { target: opened, .. } = request.as_ref()
                    && opened != &target
                {
                    return Err("scoped broadcast source mismatch".into());
                }
                if let MessageRequest::PollWindow { target: polled, .. } = request.as_ref()
                    && polled != &target
                {
                    return Err("scoped Window message target mismatch".into());
                }
                self.handle_bound(
                    caller,
                    *request,
                    live_windows,
                    origin_keys,
                    agent_origin,
                    Some(crate::shared_memory::SharedMemoryRecipient::Window {
                        page,
                        target,
                        document_generation,
                        stream_epoch,
                    }),
                )
            }
            MessageRequest::ParkFrameDocument { .. }
            | MessageRequest::RestoreFrameDocument { .. }
            | MessageRequest::DiscardFrameDocument { .. } => {
                Err("frame history requires parent policy authority".into())
            }
            MessageRequest::ParkDocument {
                document_generation,
                stream_epoch,
            } => self
                .park_document_authority(caller.window()?, document_generation, stream_epoch)
                .map(MessageResponse::DocumentParked),
            MessageRequest::RestoreDocument { token } => {
                let (document_generation, stream_epoch) =
                    self.restore_document_authority(caller.window()?, &token)?;
                Ok(MessageResponse::DocumentRestored {
                    document_generation,
                    stream_epoch,
                })
            }
            MessageRequest::DiscardDocument { token } => {
                self.discard_parked_document(caller.window()?, &token)?;
                Ok(MessageResponse::Done)
            }
            MessageRequest::RegisterRootDocument {
                document_generation,
                stream_epoch,
            } => {
                let page = caller.window()?;
                let lifetime = (document_generation, stream_epoch);
                if self.roots.get(&page) == Some(&lifetime) {
                    return Ok(MessageResponse::Done);
                }
                if self
                    .retired_roots
                    .get(&page)
                    .is_some_and(|old| *old >= lifetime)
                {
                    return Err("retired root document cannot register again".into());
                }
                if self.roots.get(&page).is_some_and(|old| *old > lifetime) {
                    return Err("stale root document registration".into());
                }
                if self.roots.len() + self.retired_roots.len() >= MAX_PORTS
                    && !self.roots.contains_key(&page)
                    && !self.retired_roots.contains_key(&page)
                {
                    return Err("root registration capacity".into());
                }
                if let Some((generation, epoch)) = self.roots.get(&page).copied() {
                    self.retire_broadcast_document(page, WindowTarget::Root, generation, epoch);
                    self.retire_active_frame_broadcasts(page);
                }
                self.roots.insert(page, lifetime);
                Ok(MessageResponse::Done)
            }
            MessageRequest::RetireRootDocument {
                document_generation,
                stream_epoch,
            } => {
                let page = caller.window()?;
                self.retire_broadcast_document(
                    page,
                    WindowTarget::Root,
                    document_generation,
                    stream_epoch,
                );
                if self.roots.get(&page) == Some(&(document_generation, stream_epoch)) {
                    self.retire_active_frame_broadcasts(page);
                    // This queue belongs to the exact active document being
                    // retired. Preserve parked and newer document queues.
                    let key = (page, WindowTarget::Root);
                    if let Some(queue) = self.windows.remove(&key) {
                        for item in queue {
                            self.discard(item.message.payload);
                        }
                    }
                    self.window_bytes.remove(&key);
                    self.roots.remove(&page);
                    let old = self
                        .retired_roots
                        .entry(page)
                        .or_insert((document_generation, stream_epoch));
                    *old = (*old).max((document_generation, stream_epoch));
                }
                Ok(MessageResponse::Done)
            }
            MessageRequest::RegisterWindow {
                frame,
                document_generation,
                stream_epoch,
                origin,
                inherit_origin,
            } => {
                let caller = caller.window()?;
                if frame.len() != 64 || !frame.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("invalid frame capability".into());
                }
                if origin.len() > 8192
                    || (origin != "null"
                        && !url::Url::parse(&origin).is_ok_and(|url| {
                            url.origin().is_tuple() && url.origin().ascii_serialization() == origin
                        }))
                {
                    return Err("invalid or oversized frame origin".into());
                }
                let origin_key = if let Some(parent) = inherit_origin {
                    self.window_origin_key(caller, &parent, live_windows, origin_keys)
                        .ok_or("inherited frame origin is retired")?
                } else if origin == "null" {
                    format!("opaque-frame:{frame}:{document_generation}")
                } else {
                    format!("oryn:origin:{origin}")
                };
                let key = (caller, frame);
                if self.frames.get(&key).is_some_and(|old| {
                    (old.document_generation, old.stream_epoch)
                        > (document_generation, stream_epoch)
                }) {
                    return Err("stale frame registration".into());
                }
                if let Some(old) = self.frames.get(&key).filter(|old| {
                    (old.document_generation, old.stream_epoch)
                        == (document_generation, stream_epoch)
                }) {
                    if old.origin != origin {
                        return Err("committed frame registration origin cannot change".into());
                    }
                    return Ok(MessageResponse::Done);
                }
                if self
                    .parked_frame_lifetimes
                    .get(&key)
                    .is_some_and(|old| *old >= (document_generation, stream_epoch))
                {
                    return Err("parked frame lifetime requires history capability".into());
                }
                if self.registered_frame_count() >= MAX_PORTS && !self.frames.contains_key(&key) {
                    return Err("frame registration capacity".into());
                }
                if let Some(old) = self.frames.get(&key) {
                    let lifetime = (old.document_generation, old.stream_epoch);
                    self.retire_broadcast_document(
                        caller,
                        WindowTarget::Frame(key.1.clone()),
                        lifetime.0,
                        lifetime.1,
                    );
                }
                self.frames.insert(
                    key,
                    WindowRegistration {
                        document_generation,
                        stream_epoch,
                        origin,
                        origin_key,
                    },
                );
                Ok(MessageResponse::Done)
            }
            MessageRequest::RetireWindow {
                frame,
                document_generation,
                stream_epoch,
            } => {
                let caller = caller.window()?;
                let key = (caller, frame.clone());
                self.retire_broadcast_document(
                    caller,
                    WindowTarget::Frame(frame.clone()),
                    document_generation,
                    stream_epoch,
                );
                if self.frames.get(&key).is_some_and(|old| {
                    (old.document_generation, old.stream_epoch)
                        == (document_generation, stream_epoch)
                }) {
                    self.frames.remove(&key);
                    let key = (caller, WindowTarget::Frame(frame));
                    if let Some(queue) = self.windows.remove(&key) {
                        for item in queue {
                            self.discard(item.message.payload)
                        }
                    }
                    self.window_bytes.remove(&key);
                }
                Ok(MessageResponse::Done)
            }
            MessageRequest::WindowStatus { page, target } => {
                caller.window()?;
                Ok(MessageResponse::WindowLive(
                    self.window_origin(page, &target, live_windows).is_some(),
                ))
            }
            MessageRequest::BroadcastOpen { target, name } => {
                if name.len() > 65_536 || self.broadcasts.len() >= MAX_PORTS {
                    return Err("BroadcastChannel capacity".into());
                }
                let (origin, origin_key) = match caller {
                    Principal::Window(page) => (
                        self.window_origin(page, &target, live_windows)
                            .ok_or("BroadcastChannel source is retired")?
                            .to_string(),
                        self.window_origin_key(page, &target, live_windows, origin_keys)
                            .ok_or("BroadcastChannel origin missing")?,
                    ),
                    Principal::Agent(_) => {
                        let (origin, key) = agent_origin.ok_or("agent origin unavailable")?;
                        (origin.to_string(), key.to_string())
                    }
                };
                let partition = self
                    .partitions
                    .get(&caller)
                    .cloned()
                    .ok_or("SecurityError: storage partition unavailable")?;
                let metadata = name
                    .len()
                    .saturating_mul(2)
                    .saturating_add(partition.len())
                    .saturating_add(origin.len())
                    .saturating_add(origin_key.len());
                let retained = self.broadcasts.values().fold(0usize, |sum, item| {
                    sum.saturating_add(item.name.len().saturating_mul(2))
                        .saturating_add(item.partition.len())
                        .saturating_add(item.origin.len())
                        .saturating_add(item.origin_key.len())
                });
                if metadata > MAX_BYTES.saturating_sub(retained) {
                    return Err("BroadcastChannel metadata capacity".into());
                }
                let id = self
                    .next
                    .checked_add(1)
                    .ok_or("broadcast identity exhausted")?;
                let secret = secret()?;
                self.next = id;
                self.broadcasts.insert(
                    id,
                    BroadcastSubscription {
                        document: document.clone(),
                        secret,
                        owner: caller,
                        partition,
                        origin_key,
                        origin,
                        name,
                        queue: VecDeque::new(),
                        bytes: 0,
                    },
                );
                Ok(MessageResponse::BroadcastOpened(PortToken { id, secret }))
            }
            MessageRequest::BroadcastPost {
                channel,
                mut payload,
            } => {
                payload.validate()?;
                if !payload.ports.is_empty() {
                    return Err("BroadcastChannel cannot transfer ports".into());
                }
                self.payload_authority(caller, &mut payload)?;
                let source = self.broadcast(caller, &channel)?;
                if source.document.is_some()
                    && (source.document != document
                        || !self.broadcast_document_active(source.document.as_ref()))
                {
                    return Err("BroadcastChannel belongs to another document lifetime".into());
                }
                let origin = source.origin.clone();
                let targets = self
                    .broadcasts
                    .iter()
                    .filter(|(id, item)| {
                        **id != channel.id
                            && self.broadcast_document_active(item.document.as_ref())
                            && item.partition == source.partition
                            && item.origin_key == source.origin_key
                            && item.name == source.name
                    })
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                let bytes = payload.retained_bytes();
                self.queue_budget(bytes.saturating_mul(targets.len()))?;
                if targets.iter().any(|id| {
                    let item = &self.broadcasts[id];
                    item.queue.len() >= MAX_MESSAGES || bytes > MAX_BYTES.saturating_sub(item.bytes)
                }) {
                    return Err("BroadcastChannel queue capacity".into());
                }
                let end = self
                    .broadcast_sequence
                    .checked_add(targets.len() as u64)
                    .ok_or("broadcast task sequence exhausted")?;
                for id in targets {
                    self.broadcast_sequence += 1;
                    let item = self.broadcasts.get_mut(&id).unwrap();
                    item.bytes += bytes;
                    item.queue.push_back((
                        self.broadcast_sequence,
                        BroadcastMessage {
                            origin: origin.clone(),
                            payload: payload.clone(),
                        },
                    ));
                }
                debug_assert_eq!(self.broadcast_sequence, end);
                Ok(MessageResponse::Done)
            }
            MessageRequest::BroadcastPoll { channel, max } => {
                let source = self.broadcast(caller, &channel)?;
                if source.document.is_some()
                    && (source.document != document
                        || !self.broadcast_document_active(source.document.as_ref()))
                {
                    return Err("BroadcastChannel belongs to another document lifetime".into());
                }
                let mut messages = Vec::new();
                // Broadcast tasks share ordering within an agent, across all
                // subscriptions. Channel-by-channel polling must not reorder
                // messages posted earlier to a later-created channel.
                for _ in 0..max.min(32) {
                    let next = self
                        .broadcasts
                        .iter()
                        .filter(|(_, item)| {
                            item.owner == caller
                                && self.broadcast_document_active(item.document.as_ref())
                        })
                        .filter_map(|(id, item)| {
                            item.queue.front().map(|(sequence, _)| (*sequence, *id))
                        })
                        .min();
                    if !next.is_some_and(|(_, id)| id == channel.id) {
                        break;
                    }
                    let item = self.broadcasts.get_mut(&channel.id).unwrap();
                    let (_, message) = item.queue.pop_front().unwrap();
                    item.bytes -= message.payload.retained_bytes();
                    messages.push(message);
                }
                for index in 0..messages.len() {
                    if let Err(error) =
                        self.receive(caller, &mut messages[index].payload, document.as_ref())
                    {
                        for message in messages {
                            self.discard(message.payload);
                        }
                        return Err(error);
                    }
                }
                Ok(MessageResponse::BroadcastMessages(messages))
            }
            MessageRequest::BroadcastClose { channel } => {
                self.broadcast(caller, &channel)?;
                self.broadcasts.remove(&channel.id);
                Ok(MessageResponse::Done)
            }
            MessageRequest::CreateChannel => Ok(MessageResponse::Channel(
                self.create_entangled(caller, caller)?,
            )),
            MessageRequest::DiscardTransfers { ports } => {
                for token in &ports {
                    self.endpoint(caller, token, true)?;
                }
                for token in ports {
                    self.close_id(token.id)
                }
                Ok(MessageResponse::Done)
            }
            MessageRequest::Transfer { ports } => {
                let mut seen = BTreeSet::new();
                for token in &ports {
                    self.endpoint(caller, token, false)?;
                    if !seen.insert(token.id) {
                        return Err("duplicate transferred port".into());
                    }
                }
                let secrets = ports
                    .iter()
                    .map(|_| secret())
                    .collect::<Result<Vec<_>, _>>()?;
                let mut result = Vec::new();
                for (mut token, secret) in ports.into_iter().zip(secrets) {
                    let endpoint = self.ports.get_mut(&token.id).unwrap();
                    endpoint.owner = Owner::Transit(caller);
                    endpoint.secret = secret;
                    token.secret = secret;
                    result.push(token)
                }
                Ok(MessageResponse::Transferred(result))
            }
            MessageRequest::PostPort { port, mut payload } => {
                let peer = self.endpoint(caller, &port, false)?.peer;
                self.payload_authority(caller, &mut payload)?;
                if payload.ports.iter().any(|p| p.id == port.id) {
                    return Err("cannot transfer sending port".into());
                }
                let bytes = payload.retained_bytes();
                if let Err(error) = self.queue_budget(bytes) {
                    self.discard(payload);
                    return Err(error);
                }
                if let Some(peer) = peer.filter(|peer| !payload.ports.iter().any(|p| p.id == *peer))
                {
                    let endpoint = self.ports.get_mut(&peer).ok_or("missing peer")?;
                    if endpoint.closed {
                        self.discard(payload);
                        return Ok(MessageResponse::Done);
                    }
                    if endpoint.queue.len() >= MAX_MESSAGES
                        || bytes > MAX_BYTES.saturating_sub(endpoint.bytes)
                    {
                        self.discard(payload);
                        return Err("MessagePort queue capacity".into());
                    }
                    self.enqueue_transfers(&payload);
                    let endpoint = self.ports.get_mut(&peer).unwrap();
                    endpoint.bytes += bytes;
                    endpoint.queue.push_back(payload);
                } else {
                    self.discard(payload)
                }
                Ok(MessageResponse::Done)
            }
            MessageRequest::PortPending { port } => {
                let endpoint = self.endpoint(caller, &port, false)?;
                Ok(MessageResponse::PortPending(
                    !endpoint.queue.is_empty() || endpoint.close_pending,
                ))
            }
            MessageRequest::PollPort { port, max } => {
                self.endpoint(caller, &port, false)?;
                let mut messages = Vec::new();
                for _ in 0..max.min(32) {
                    let endpoint = self.ports.get_mut(&port.id).unwrap();
                    let Some(mut value) = endpoint.queue.pop_front() else {
                        break;
                    };
                    endpoint.bytes -= value.retained_bytes();
                    if let Err(error) = self.receive(caller, &mut value, document.as_ref()) {
                        self.discard(value);
                        for previous in messages {
                            self.discard(previous);
                        }
                        return Err(error);
                    }
                    messages.push(value)
                }
                let endpoint = self.ports.get_mut(&port.id).unwrap();
                let peer_closed =
                    endpoint.queue.is_empty() && std::mem::take(&mut endpoint.close_pending);
                Ok(MessageResponse::PortMessages {
                    messages,
                    peer_closed,
                })
            }
            MessageRequest::ClosePort { port } => {
                self.endpoint(caller, &port, false)?;
                self.close_id(port.id);
                Ok(MessageResponse::Done)
            }
            MessageRequest::PostWindow {
                target_page,
                target,
                source,
                target_origin,
                mut payload,
            } => {
                self.payload_authority(caller, &mut payload)?;
                let caller_page = caller.window()?;
                let source_origin = self
                    .window_origin(caller_page, &source, live_windows)
                    .ok_or("source frame is retired")?
                    .to_string();
                if self
                    .window_origin(target_page, &target, live_windows)
                    .is_none()
                {
                    self.discard(payload);
                    return Ok(MessageResponse::Done);
                }
                let required_origin = match target_origin {
                    TargetOrigin::Any => None,
                    TargetOrigin::SameOrigin => Some(
                        self.window_origin_key(caller_page, &source, live_windows, origin_keys)
                            .ok_or("source origin retired")?,
                    ),
                    TargetOrigin::Tuple(origin) => Some(format!("oryn:origin:{origin}")),
                    TargetOrigin::Never => Some("never".into()),
                };
                let key = (target_page, target);
                let bytes = payload.retained_bytes();
                if let Err(error) = self.queue_budget(bytes) {
                    self.discard(payload);
                    return Err(error);
                }
                let retained = self.window_bytes.entry(key.clone()).or_default();
                if self.windows.get(&key).map_or(0, |q| q.len()) >= MAX_MESSAGES
                    || bytes > MAX_BYTES.saturating_sub(*retained)
                {
                    self.discard(payload);
                    return Err("Window message queue capacity".into());
                }
                *retained += bytes;
                self.enqueue_transfers(&payload);
                self.windows
                    .entry(key)
                    .or_default()
                    .push_back(QueuedWindow {
                        required_origin,
                        message: WindowMessage {
                            source_page: caller_page,
                            source,
                            source_origin,
                            payload,
                        },
                    });
                Ok(MessageResponse::Done)
            }
            MessageRequest::PollWindow { target, max } => {
                let caller_page = caller.window()?;
                let origin_key = self
                    .window_origin_key(caller_page, &target, live_windows, origin_keys)
                    .ok_or("target frame is retired")?;
                let key = (caller_page, target);
                let mut messages: Vec<WindowMessage> = Vec::new();
                for _ in 0..max.min(32) {
                    let Some(mut item) = self.windows.entry(key.clone()).or_default().pop_front()
                    else {
                        break;
                    };
                    *self.window_bytes.entry(key.clone()).or_default() -=
                        item.message.payload.retained_bytes();
                    let matches = item
                        .required_origin
                        .as_ref()
                        .is_none_or(|required| required == &origin_key);
                    if !matches {
                        self.discard(item.message.payload);
                        continue;
                    }
                    if let Err(error) =
                        self.receive(caller, &mut item.message.payload, document.as_ref())
                    {
                        self.discard(item.message.payload);
                        for previous in messages {
                            self.discard(previous.payload);
                        }
                        return Err(error);
                    }
                    messages.push(item.message);
                }
                Ok(MessageResponse::WindowMessages(messages))
            }
        }
    }
    pub fn retire_page(&mut self, page: u64) {
        self.cancel_page_prerenders(page);
        if let Some(backend) = &self.shared_backend {
            backend.retire_principal(Principal::Window(page));
        }
        let tokens = self
            .parked
            .iter()
            .filter_map(|(token, document)| {
                (document.page == page).then_some(ParkedDocumentToken(*token))
            })
            .collect::<Vec<_>>();
        for token in tokens {
            let _ = self.discard_parked_document(page, &token);
        }
        self.roots.remove(&page);
        self.retired_roots.remove(&page);
        self.parked_frame_lifetimes
            .retain(|(owner, _), _| *owner != page);
        let owned = self
            .ports
            .iter()
            .filter_map(|(id, p)| {
                (p.owner == Owner::Bound(Principal::Window(page))
                    || p.owner == Owner::Transit(Principal::Window(page)))
                .then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in owned {
            self.close_id(id)
        }
        let windows = self
            .windows
            .keys()
            .filter(|(owner, _)| *owner == page)
            .cloned()
            .collect::<Vec<_>>();
        for key in windows {
            if let Some(queue) = self.windows.remove(&key) {
                for item in queue {
                    self.discard(item.message.payload)
                }
            }
            self.window_bytes.remove(&key);
        }
        self.frames.retain(|(owner, _), _| *owner != page);
        self.broadcasts
            .retain(|_, channel| channel.owner != Principal::Window(page));
        self.partitions.remove(&Principal::Window(page));
    }
}

impl std::fmt::Debug for MessageHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageHub")
            .field("endpoint_count", &self.ports.len())
            .field("window_queue_count", &self.windows.len())
            .finish_non_exhaustive()
    }
}
impl MessageHub {
    pub fn pending_pages(&self) -> Vec<u64> {
        self.windows
            .iter()
            .filter_map(|((page, _), q)| (!q.is_empty()).then_some(*page))
            .chain(self.ports.values().filter_map(|p| match p.owner {
                Owner::Bound(Principal::Window(page)) if !p.queue.is_empty() || p.close_pending => {
                    Some(page)
                }
                _ => None,
            }))
            .chain(
                self.broadcasts
                    .values()
                    .filter_map(|item| match item.owner {
                        Principal::Window(page) if !item.queue.is_empty() => Some(page),
                        _ => None,
                    }),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pages() -> BTreeMap<u64, String> {
        [
            (1, "https://one.test".into()),
            (2, "https://two.test".into()),
            (3, "null".into()),
        ]
        .into()
    }
    fn value(ports: Vec<PortToken>) -> WireValue {
        WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![1, 2, 3],
            hosts: vec![],
            buffers: vec![],
            ports,
        }
    }
    fn channel(hub: &mut MessageHub, page: u64) -> [PortToken; 2] {
        let MessageResponse::Channel(ports) = hub
            .handle(page, MessageRequest::CreateChannel, &pages())
            .unwrap()
        else {
            panic!()
        };
        ports
    }
    fn transfer(hub: &mut MessageHub, page: u64, ports: Vec<PortToken>) -> Vec<PortToken> {
        let MessageResponse::Transferred(ports) = hub
            .handle(page, MessageRequest::Transfer { ports }, &pages())
            .unwrap()
        else {
            panic!()
        };
        ports
    }
    #[test]
    fn cached_token_authentication_survives_consumption_without_owner_tombstones() {
        let mut hub = MessageHub::default();
        let token = ParkedDocumentToken(hub.mint_park_token(7).unwrap());
        assert!(hub.cached_token_expired(7, &token));
        assert!(!hub.cached_token_expired(8, &token));
        let mut forged = token.clone();
        forged.0[31] ^= 1;
        assert!(!hub.cached_token_expired(7, &forged));
        let mut forged_owner = token.clone();
        forged_owner.0[..8].copy_from_slice(&8u64.to_be_bytes());
        assert!(!hub.cached_token_expired(8, &forged_owner));
        assert!(!MessageHub::default().cached_token_expired(7, &token));
        hub.park_token_sequence = u64::MAX;
        assert!(hub.mint_park_token(7).is_err());
        assert!(hub.cached_token_expired(7, &token));
    }
    #[test]
    fn transfer_is_atomic_one_use_and_survives_sender_retirement_after_delivery() {
        let mut hub = MessageHub::default();
        let [a, b] = channel(&mut hub, 1);
        assert!(
            hub.handle(
                1,
                MessageRequest::Transfer {
                    ports: vec![b.clone(), b.clone()]
                },
                &pages()
            )
            .is_err()
        );
        let ticket = transfer(&mut hub, 1, vec![b.clone()]).remove(0);
        assert!(
            hub.handle(1, MessageRequest::PollPort { port: b, max: 1 }, &pages())
                .is_err()
        );
        let post = MessageRequest::PostWindow {
            target_page: 2,
            target: WindowTarget::Root,
            source: WindowTarget::Root,
            target_origin: TargetOrigin::Any,
            payload: value(vec![ticket.clone()]),
        };
        hub.handle(1, post.clone(), &pages()).unwrap();
        assert!(hub.handle(1, post, &pages()).is_err());
        let MessageResponse::WindowMessages(mut messages) = hub
            .handle(
                2,
                MessageRequest::PollWindow {
                    target: WindowTarget::Root,
                    max: 1,
                },
                &pages(),
            )
            .unwrap()
        else {
            panic!()
        };
        let received = messages.remove(0).payload.ports.remove(0);
        assert_ne!(received.secret, ticket.secret);
        assert!(
            hub.handle(
                2,
                MessageRequest::PollPort {
                    port: ticket,
                    max: 1
                },
                &pages()
            )
            .is_err()
        );
        hub.handle(
            1,
            MessageRequest::PostPort {
                port: a,
                payload: value(vec![]),
            },
            &pages(),
        )
        .unwrap();
        hub.retire_page(1);
        let MessageResponse::PortMessages {
            messages,
            peer_closed,
        } = hub
            .handle(
                2,
                MessageRequest::PollPort {
                    port: received.clone(),
                    max: 1,
                },
                &pages(),
            )
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(messages.len(), 1);
        assert!(peer_closed);
        let MessageResponse::PortMessages { peer_closed, .. } = hub
            .handle(
                2,
                MessageRequest::PollPort {
                    port: received,
                    max: 1,
                },
                &pages(),
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(!peer_closed);
    }
    #[test]
    fn broadcast_names_have_aggregate_native_metadata_custody() {
        let mut hub = MessageHub::default();
        hub.set_partition(Principal::Window(1), "partition".into());
        let mut tokens = Vec::new();
        let mut refused = false;
        for _ in 0..512 {
            let before = hub.next;
            match hub.handle(
                1,
                MessageRequest::BroadcastOpen {
                    target: WindowTarget::Root,
                    name: vec![65; 65536],
                },
                &pages(),
            ) {
                Ok(MessageResponse::BroadcastOpened(token)) => tokens.push(token),
                Err(error) => {
                    assert!(error.contains("metadata capacity"));
                    assert_eq!(hub.next, before);
                    refused = true;
                    break;
                }
                _ => panic!(),
            }
        }
        assert!(refused);
        let count = hub.broadcasts.len();
        hub.handle(
            1,
            MessageRequest::BroadcastClose {
                channel: tokens.pop().unwrap(),
            },
            &pages(),
        )
        .unwrap();
        assert_eq!(hub.broadcasts.len(), count - 1);
        assert!(matches!(
            hub.handle(
                1,
                MessageRequest::BroadcastOpen {
                    target: WindowTarget::Root,
                    name: vec![65; 65536]
                },
                &pages()
            )
            .unwrap(),
            MessageResponse::BroadcastOpened(_)
        ));
    }
    #[test]
    fn frame_registration_rejects_noncanonical_or_unbounded_principals_before_mutation() {
        let mut hub = MessageHub::default();
        let frame = "cd".repeat(32);
        for origin in [
            "https://frame.test/path".into(),
            "https://user@frame.test".into(),
            "https://FRAME.test".into(),
            "data:,opaque".into(),
            "x".repeat(8193),
        ] {
            assert!(
                hub.handle(
                    1,
                    MessageRequest::RegisterWindow {
                        frame: frame.clone(),
                        document_generation: 1,
                        stream_epoch: 0,
                        origin,
                        inherit_origin: None
                    },
                    &pages()
                )
                .is_err()
            );
            assert!(hub.frames.is_empty());
        }
        hub.handle(
            1,
            MessageRequest::RegisterWindow {
                frame,
                document_generation: 1,
                stream_epoch: 0,
                origin: "https://frame.test".into(),
                inherit_origin: None,
            },
            &pages(),
        )
        .unwrap();
        assert_eq!(hub.frames.len(), 1);
    }
    #[test]
    fn frame_queue_cannot_be_drained_by_root_and_old_retirement_cannot_revoke_replacement() {
        let mut hub = MessageHub::default();
        let frame = "ab".repeat(32);
        let target = WindowTarget::Frame(frame.clone());
        let register = |generation, origin: &str| MessageRequest::RegisterWindow {
            frame: frame.clone(),
            document_generation: generation,
            stream_epoch: 0,
            origin: origin.into(),
            inherit_origin: None,
        };
        hub.handle(2, register(1, "https://frame.test"), &pages())
            .unwrap();
        hub.handle(
            1,
            MessageRequest::PostWindow {
                target_page: 2,
                target: target.clone(),
                source: WindowTarget::Root,
                target_origin: TargetOrigin::Tuple("https://frame.test".into()),
                payload: value(vec![]),
            },
            &pages(),
        )
        .unwrap();
        let MessageResponse::WindowMessages(messages) = hub
            .handle(
                2,
                MessageRequest::PollWindow {
                    target: WindowTarget::Root,
                    max: 32,
                },
                &pages(),
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(messages.is_empty());
        hub.handle(2, register(2, "https://new.test"), &pages())
            .unwrap();
        hub.handle(
            2,
            MessageRequest::RetireWindow {
                frame: frame.clone(),
                document_generation: 1,
                stream_epoch: 0,
            },
            &pages(),
        )
        .unwrap();
        assert!(matches!(
            hub.handle(
                1,
                MessageRequest::WindowStatus {
                    page: 2,
                    target: target.clone()
                },
                &pages()
            )
            .unwrap(),
            MessageResponse::WindowLive(true)
        ));
        let MessageResponse::WindowMessages(messages) = hub
            .handle(
                2,
                MessageRequest::PollWindow {
                    target: target.clone(),
                    max: 32,
                },
                &pages(),
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(messages.is_empty());
        hub.handle(
            2,
            MessageRequest::RetireWindow {
                frame,
                document_generation: 2,
                stream_epoch: 0,
            },
            &pages(),
        )
        .unwrap();
        assert!(matches!(
            hub.handle(
                1,
                MessageRequest::WindowStatus { page: 2, target },
                &pages()
            )
            .unwrap(),
            MessageResponse::WindowLive(false)
        ));
    }
    #[test]
    fn foreign_capabilities_and_malformed_payloads_fail_without_mutation() {
        let mut hub = MessageHub::default();
        let [a, b] = channel(&mut hub, 1);
        assert!(
            hub.handle(2, MessageRequest::ClosePort { port: a.clone() }, &pages())
                .is_err()
        );
        let mut forged = a.clone();
        forged.secret[0] ^= 1;
        assert!(
            hub.handle(1, MessageRequest::ClosePort { port: forged }, &pages())
                .is_err()
        );
        let mut malformed = value(vec![]);
        malformed.hosts.push(WireHost::Port(9));
        assert!(malformed.validate().is_err());
        hub.handle(
            1,
            MessageRequest::PostPort {
                port: a,
                payload: value(vec![]),
            },
            &pages(),
        )
        .unwrap();
        assert!(
            matches!(hub.handle(1,MessageRequest::PollPort{port:b,max:1},&pages()).unwrap(),MessageResponse::PortMessages{messages,..}if messages.len()==1)
        );
        let debug = format!("{hub:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("https://"));
    }
    #[test]
    fn dropped_destination_disposes_transfers_and_frees_capacity() {
        let mut hub = MessageHub::default();
        let [a, b] = channel(&mut hub, 1);
        let ports = transfer(&mut hub, 1, vec![b]);
        hub.handle(
            1,
            MessageRequest::PostWindow {
                target_page: 999,
                target: WindowTarget::Root,
                source: WindowTarget::Root,
                target_origin: TargetOrigin::Any,
                payload: value(ports),
            },
            &pages(),
        )
        .unwrap();
        assert_eq!(hub.ports.len(), 1);
        assert!(matches!(
            hub.handle(1, MessageRequest::PollPort { port: a, max: 1 }, &pages())
                .unwrap(),
            MessageResponse::PortMessages {
                peer_closed: true,
                ..
            }
        ));
        hub.retire_page(1);
        assert!(hub.ports.is_empty());
    }
}

#[cfg(test)]
mod opaque_origin_tests {
    use super::*;
    #[test]
    fn same_origin_uses_parent_identity_not_serialized_null() {
        let live = [(1, "null".into()), (2, "null".into()), (3, "null".into())].into();
        let keys = [
            (1, "opaque-shared".into()),
            (2, "opaque-shared".into()),
            (3, "opaque-other".into()),
        ]
        .into();
        let mut hub = MessageHub::default();
        for target in [2, 3] {
            hub.handle_with_origin_keys(
                1,
                MessageRequest::PostWindow {
                    target_page: target,
                    target: WindowTarget::Root,
                    source: WindowTarget::Root,
                    target_origin: TargetOrigin::SameOrigin,
                    payload: WireValue {
                        shared: Vec::new(),
                        shared_queue: None,
                        shared_recipient: None,
                        wasm_modules: Vec::new(),
                        version: 1,
                        bytes: vec![0],
                        hosts: vec![],
                        buffers: vec![],
                        ports: vec![],
                    },
                },
                &live,
                &keys,
            )
            .unwrap();
        }
        let MessageResponse::WindowMessages(same) = hub
            .handle_with_origin_keys(
                2,
                MessageRequest::PollWindow {
                    target: WindowTarget::Root,
                    max: 1,
                },
                &live,
                &keys,
            )
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(same.len(), 1);
        let MessageResponse::WindowMessages(other) = hub
            .handle_with_origin_keys(
                3,
                MessageRequest::PollWindow {
                    target: WindowTarget::Root,
                    max: 1,
                },
                &live,
                &keys,
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(other.is_empty());
        let frame = "cd".repeat(32);
        hub.handle_with_origin_keys(
            2,
            MessageRequest::RegisterWindow {
                frame: frame.clone(),
                document_generation: 1,
                stream_epoch: 0,
                origin: "null".into(),
                inherit_origin: Some(WindowTarget::Root),
            },
            &live,
            &keys,
        )
        .unwrap();
        assert_eq!(
            hub.window_origin_key(2, &WindowTarget::Frame(frame), &live, &keys),
            Some("opaque-shared".into())
        );
    }
}

impl MessageHub {
    pub fn retire_agent(&mut self, agent: u64) {
        if let Some(backend) = &self.shared_backend {
            backend.retire_principal(Principal::Agent(agent));
        }
        let ids = self
            .ports
            .iter()
            .filter_map(|(id, endpoint)| {
                (endpoint.owner == Owner::Bound(Principal::Agent(agent))
                    || endpoint.owner == Owner::Transit(Principal::Agent(agent)))
                .then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in ids {
            self.close_id(id)
        }
        self.broadcasts
            .retain(|_, channel| channel.owner != Principal::Agent(agent));
        self.partitions.remove(&Principal::Agent(agent));
    }
    pub fn pending_agents(&self) -> Vec<u64> {
        self.ports
            .values()
            .filter_map(|port| match port.owner {
                Owner::Bound(Principal::Agent(agent))
                    if !port.queue.is_empty() || port.close_pending =>
                {
                    Some(agent)
                }
                _ => None,
            })
            .chain(
                self.broadcasts
                    .values()
                    .filter_map(|item| match item.owner {
                        Principal::Agent(agent) if !item.queue.is_empty() => Some(agent),
                        _ => None,
                    }),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

impl MessageHub {
    /// Parent-only delivery between authenticated Window/Agent principals. The
    /// receiving capability rotation is complete before the payload is exposed.
    pub fn route_value(
        &mut self,
        sender: Principal,
        recipient: Principal,
        value: &mut WireValue,
    ) -> Result<(), String> {
        self.route_value_in_document(sender, recipient, None, value)
    }
    pub fn retain_value(&self, sender: Principal, value: &mut WireValue) -> Result<(), String> {
        self.payload_authority(sender, value)
    }
    pub fn route_value_for_document(
        &mut self,
        sender: Principal,
        recipient: crate::shared_memory::SharedMemoryRecipient,
        value: &mut WireValue,
    ) -> Result<(), String> {
        self.route_value_in_document(sender, recipient.principal(), Some(&recipient), value)
    }
    fn route_value_in_document(
        &mut self,
        sender: Principal,
        recipient: Principal,
        document: Option<&crate::shared_memory::SharedMemoryRecipient>,
        value: &mut WireValue,
    ) -> Result<(), String> {
        self.payload_authority(sender, value)?;
        self.enqueue_transfers(value);
        if let Err(error) = self.receive(recipient, value, document) {
            for token in &value.ports {
                if let Some(endpoint) = self.ports.get_mut(&token.id)
                    && endpoint.secret == token.secret
                    && endpoint.owner == Owner::Queued
                {
                    endpoint.owner = Owner::Transit(sender)
                }
            }
            return Err(error);
        }
        Ok(())
    }
    /// Parent-only channel creation for already authenticated principals. This
    /// avoids inventing a Window identity when attaching a SharedWorker client.
    pub fn create_entangled(
        &mut self,
        first: Principal,
        second: Principal,
    ) -> Result<[PortToken; 2], String> {
        if self.ports.len() > MAX_PORTS - 2 {
            return Err("MessagePort capacity".into());
        }
        let a = self
            .next
            .checked_add(1)
            .ok_or("MessagePort identity exhausted")?;
        let b = a.checked_add(1).ok_or("MessagePort identity exhausted")?;
        // Allocate both secrets before publishing either endpoint.
        let sa = secret()?;
        let sb = secret()?;
        self.next = b;
        for (id, secret, peer, owner) in [(a, sa, b, first), (b, sb, a, second)] {
            self.ports.insert(
                id,
                Endpoint {
                    secret,
                    owner: Owner::Bound(owner),
                    peer: Some(peer),
                    closed: false,
                    close_pending: false,
                    queue: VecDeque::new(),
                    bytes: 0,
                },
            );
        }
        Ok([
            PortToken { id: a, secret: sa },
            PortToken { id: b, secret: sb },
        ])
    }
    /// Discard a remapped payload when a bounded process queue rejects delivery.
    pub fn validate_received_value(
        &self,
        recipient: Principal,
        value: &WireValue,
    ) -> Result<(), String> {
        value.validate()?;
        for token in &value.ports {
            self.endpoint(recipient, token, false)?;
        }
        Ok(())
    }
    pub fn release_value(&mut self, recipient: Principal, value: WireValue) -> Result<(), String> {
        self.dispose_queued_value(recipient, value)
    }
    /// Parent-owned failed admission may dispose transit tokens after source
    /// retirement. Live capabilities must still match the authenticated sender.
    pub fn discard_transit_value(
        &mut self,
        sender: Principal,
        value: WireValue,
    ) -> Result<(), String> {
        for token in &value.ports {
            if let Some(endpoint) = self.ports.get(&token.id) {
                if endpoint.closed {
                    continue;
                }
                self.endpoint(sender, token, true)?;
            }
        }
        if !value.shared.is_empty() {
            self.shared_backend
                .as_ref()
                .ok_or("shared message backend unavailable")?
                .discard_offered(sender, &value.shared)?;
        }
        for token in value.ports {
            self.close_id(token.id);
        }
        Ok(())
    }
    /// Parent teardown may race with page/agent principal retirement. Missing
    /// endpoints are already disposed; every surviving endpoint must still
    /// match the queued recipient and capability before any are closed.
    /// Parent response compensation remains idempotent after principal close.
    pub fn discard_broadcast(&mut self, owner: Principal, token: &PortToken) -> Result<(), String> {
        if self.broadcasts.contains_key(&token.id) {
            self.broadcast(owner, token)?;
            self.broadcasts.remove(&token.id);
        }
        Ok(())
    }
    /// Retry exact shared delivery claims retained by failed queue cleanup.
    /// Each handoff is independent: one rejected claim cannot prevent releasing
    /// another client's claim for the same backing allocation.
    pub fn retry_shared_disposals(&mut self) -> Result<(), String> {
        let mut failure = None;
        let pending = std::mem::take(&mut self.pending_shared_disposals);
        for ((recipient, handoff), (backend, reference)) in pending {
            let backend = backend.or_else(|| self.shared_backend.clone());
            let result = backend
                .as_ref()
                .ok_or_else(|| "shared message backend unavailable".to_string())
                .and_then(|backend| backend.discard_delivered(recipient, &[reference]));
            if let Err(error) = result {
                self.pending_shared_disposals
                    .insert((recipient, handoff), (backend, reference));
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
    fn dispose_shared_deliveries(
        &mut self,
        recipient: Principal,
        references: &[crate::shared_memory::SharedMemoryReference],
    ) -> Result<(), String> {
        let mut failure = None;
        for reference in references {
            let key = (recipient, reference.handoff);
            if self
                .pending_shared_disposals
                .get(&key)
                .is_some_and(|(_, owned)| owned.descriptor != reference.descriptor)
            {
                failure.get_or_insert("shared cleanup identity mismatch".to_string());
                continue;
            }
            let backend = self
                .pending_shared_disposals
                .get(&key)
                .and_then(|(backend, _)| backend.clone())
                .or_else(|| self.shared_backend.clone());
            let result = backend
                .as_ref()
                .ok_or_else(|| "shared message backend unavailable".to_string())
                .and_then(|backend| backend.discard_delivered(recipient, &[*reference]));
            match result {
                Ok(()) => {
                    self.pending_shared_disposals.remove(&key);
                }
                Err(error) => {
                    self.pending_shared_disposals
                        .insert(key, (backend, *reference));
                    failure.get_or_insert(error);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
    pub fn dispose_queued_value(
        &mut self,
        recipient: Principal,
        value: WireValue,
    ) -> Result<(), String> {
        for token in &value.ports {
            if self.ports.contains_key(&token.id) {
                self.endpoint(recipient, token, false)?;
            }
        }
        let result = self.dispose_shared_deliveries(recipient, &value.shared);
        // Validated port custody commits independently of a backend's shared
        // memory release error. Shared failures stay owned by this hub.
        for token in value.ports {
            self.close_id(token.id);
        }
        result
    }
}
#[cfg(test)]
mod broadcast_tests {
    use super::*;
    fn payload() -> WireValue {
        WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![1],
            hosts: vec![],
            buffers: vec![],
            ports: vec![],
        }
    }
    #[test]
    fn queued_disposal_tolerates_retired_ports_but_preserves_foreign_live_ports() {
        let mut hub = MessageHub::default();
        let [retired, _] = hub
            .create_entangled(Principal::Window(1), Principal::Agent(1))
            .unwrap();
        let [owned, _] = hub
            .create_entangled(Principal::Window(1), Principal::Agent(2))
            .unwrap();
        let [foreign, _] = hub
            .create_entangled(Principal::Window(2), Principal::Agent(3))
            .unwrap();
        let wire = |ports| WireValue { ports, ..payload() };
        hub.release_value(Principal::Window(1), wire(vec![retired.clone()]))
            .unwrap();
        assert!(
            hub.dispose_queued_value(
                Principal::Window(1),
                wire(vec![retired.clone(), owned.clone(), foreign.clone()])
            )
            .is_err()
        );
        assert!(
            hub.endpoint(Principal::Window(1), &owned, false).is_ok(),
            "rejection must be atomic for live endpoints"
        );
        assert!(hub.endpoint(Principal::Window(2), &foreign, false).is_ok());
        hub.dispose_queued_value(
            Principal::Window(1),
            wire(vec![retired.clone(), owned.clone()]),
        )
        .unwrap();
        hub.dispose_queued_value(Principal::Window(1), wire(vec![retired, owned]))
            .unwrap();
        assert!(hub.endpoint(Principal::Window(2), &foreign, false).is_ok());
    }
    #[test]
    fn cached_frame_token_is_target_bound_and_eviction_does_not_retire_root() {
        let live = BTreeMap::from([
            (1, "https://root.test".into()),
            (2, "https://root.test".into()),
        ]);
        let mut hub = MessageHub::default();
        hub.handle(
            1,
            MessageRequest::RegisterRootDocument {
                document_generation: 1,
                stream_epoch: 0,
            },
            &live,
        )
        .unwrap();
        let frame = "e".repeat(64);
        let register = || MessageRequest::RegisterWindow {
            frame: frame.clone(),
            document_generation: 3,
            stream_epoch: 0,
            origin: "null".into(),
            inherit_origin: None,
        };
        hub.handle(1, register(), &live).unwrap();
        let token = hub
            .park_frame_authority(1, &frame, 3, 0, &[(frame.clone(), 3, 0)])
            .unwrap();
        assert!(hub.root_document_authority(1, 1, 0).is_ok());
        assert!(hub.handle(1, register(), &live).is_err());
        assert!(hub.restore_frame_authority(2, &frame, &token).is_err());
        assert!(
            hub.restore_frame_authority(1, &"f".repeat(64), &token)
                .is_err()
        );
        assert!(hub.restore_document_authority(1, &token).is_err());
        hub.discard_parked_document(1, &token).unwrap();
        hub.discard_parked_document(1, &token).unwrap();
        assert!(hub.root_document_authority(1, 1, 0).is_ok());
        assert!(hub.handle(1, register(), &live).is_err());
        assert!(hub.restore_frame_authority(1, &frame, &token).is_err());
    }

    #[test]
    fn cached_document_authority_requires_one_use_native_token() {
        let live = BTreeMap::from([
            (1, "https://root.test".into()),
            (2, "https://root.test".into()),
        ]);
        let mut hub = MessageHub::default();
        let register = |generation| MessageRequest::RegisterRootDocument {
            document_generation: generation,
            stream_epoch: 0,
        };
        hub.handle(1, register(3), &live).unwrap();
        let frame = "a".repeat(64);
        hub.handle(
            1,
            MessageRequest::RegisterWindow {
                frame: frame.clone(),
                document_generation: 3,
                stream_epoch: 0,
                origin: "null".into(),
                inherit_origin: None,
            },
            &live,
        )
        .unwrap();
        let origin = hub.frame_document_authority(1, &frame, 3, 0).unwrap();
        let old = hub.park_document_authority(1, 3, 0).unwrap();
        assert!(hub.root_document_authority(1, 3, 0).is_err());
        assert!(hub.frame_document_authority(1, &frame, 3, 0).is_err());
        assert!(hub.handle(1, register(3), &live).is_err());
        assert!(hub.restore_document_authority(2, &old).is_err());
        hub.handle(1, register(4), &live).unwrap();
        assert!(hub.restore_document_authority(1, &old).is_err());
        let newer = hub.park_document_authority(1, 4, 0).unwrap();
        assert_eq!(hub.restore_document_authority(1, &old).unwrap(), (3, 0));
        assert_eq!(
            hub.frame_document_authority(1, &frame, 3, 0).unwrap(),
            origin
        );
        hub.handle(1, register(3), &live).unwrap(); // idempotent live registration
        assert!(hub.restore_document_authority(1, &old).is_err());
        let older_again = hub.park_document_authority(1, 3, 0).unwrap();
        assert!(hub.handle(1, register(4), &live).is_err()); // high-water survives restoration
        hub.restore_document_authority(1, &newer).unwrap();
        hub.retire_page(1);
        assert!(hub.restore_document_authority(1, &older_again).is_err());
    }

    #[test]
    fn cached_window_queues_stay_budgeted_and_eviction_closes_transferred_ports() {
        let live = BTreeMap::from([(1, "https://root.test".into())]);
        let mut hub = MessageHub::default();
        hub.handle(
            1,
            MessageRequest::RegisterRootDocument {
                document_generation: 1,
                stream_epoch: 0,
            },
            &live,
        )
        .unwrap();
        let MessageResponse::Channel([sender, other]) =
            hub.handle(1, MessageRequest::CreateChannel, &live).unwrap()
        else {
            panic!()
        };
        let MessageResponse::Transferred(ports) = hub
            .handle(1, MessageRequest::Transfer { ports: vec![other] }, &live)
            .unwrap()
        else {
            panic!()
        };
        let payload = WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![1, 2, 3],
            hosts: vec![WireHost::Port(0)],
            buffers: vec![],
            ports,
        };
        hub.handle(
            1,
            MessageRequest::PostWindow {
                target_page: 1,
                target: WindowTarget::Root,
                source: WindowTarget::Root,
                target_origin: TargetOrigin::Any,
                payload,
            },
            &live,
        )
        .unwrap();
        let bytes = hub.retained_bytes();
        assert!(bytes > 0);
        let token = hub.park_document_authority(1, 1, 0).unwrap();
        assert_eq!(hub.retained_bytes(), bytes);
        assert!(hub.root_document_authority(1, 1, 0).is_err());
        assert!(
            hub.retained_document_authority(1, &WindowTarget::Root, 1, 0)
                .is_ok()
        );
        assert!(
            hub.retained_document_authority(1, &WindowTarget::Root, 2, 0)
                .is_err()
        );
        let MessageResponse::WindowMessages(messages) = hub
            .handle(
                1,
                MessageRequest::PollWindow {
                    target: WindowTarget::Root,
                    max: 10,
                },
                &live,
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(messages.is_empty());
        hub.discard_parked_document(1, &token).unwrap();
        hub.discard_parked_document(1, &token).unwrap();
        assert_eq!(hub.retained_bytes(), 0);
        assert!(matches!(
            hub.handle(
                1,
                MessageRequest::PollPort {
                    port: sender,
                    max: 1
                },
                &live
            )
            .unwrap(),
            MessageResponse::PortMessages {
                peer_closed: true,
                ..
            }
        ));
        assert!(hub.restore_document_authority(1, &token).is_err());
        assert!(
            hub.retained_document_authority(1, &WindowTarget::Root, 1, 0)
                .is_err()
        );
    }

    #[test]
    fn parked_document_wire_is_native_window_only_and_keeps_failed_restore_capability() {
        let live = BTreeMap::from([
            (1, "https://root.test".into()),
            (2, "https://other.test".into()),
        ]);
        let mut hub = MessageHub::default();
        hub.handle(
            1,
            MessageRequest::RegisterRootDocument {
                document_generation: 2,
                stream_epoch: 3,
            },
            &live,
        )
        .unwrap();
        let MessageResponse::DocumentParked(token) = hub
            .handle(
                1,
                MessageRequest::ParkDocument {
                    document_generation: 2,
                    stream_epoch: 3,
                },
                &live,
            )
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(format!("{token:?}"), "ParkedDocumentToken([redacted])");
        assert!(
            hub.handle_agent(
                4,
                MessageRequest::RestoreDocument {
                    token: token.clone()
                },
                "https://root.test",
                "root-key"
            )
            .is_err()
        );
        assert!(
            hub.handle(
                2,
                MessageRequest::RestoreDocument {
                    token: token.clone()
                },
                &live
            )
            .is_err()
        );
        assert!(matches!(
            hub.handle(
                1,
                MessageRequest::RestoreDocument {
                    token: token.clone()
                },
                &live
            )
            .unwrap(),
            MessageResponse::DocumentRestored {
                document_generation: 2,
                stream_epoch: 3
            }
        ));
        assert!(
            hub.handle(1, MessageRequest::RestoreDocument { token }, &live)
                .is_err()
        );
    }

    #[test]
    fn exact_root_retirement_discards_queued_transfers_and_preserves_parked_and_newer_queues() {
        let live = BTreeMap::from([(1, "https://root.test".into())]);
        let mut hub = MessageHub::default();
        let register = |generation| MessageRequest::RegisterRootDocument {
            document_generation: generation,
            stream_epoch: 0,
        };
        let retire = |generation| MessageRequest::RetireRootDocument {
            document_generation: generation,
            stream_epoch: 0,
        };
        let post = |payload| MessageRequest::PostWindow {
            target_page: 1,
            target: WindowTarget::Root,
            source: WindowTarget::Root,
            target_origin: TargetOrigin::Any,
            payload,
        };
        hub.handle(1, register(1), &live).unwrap();
        hub.handle(1, post(payload()), &live).unwrap();
        let parked_bytes = hub.retained_bytes();
        let parked = hub.park_document_authority(1, 1, 0).unwrap();
        hub.handle(1, register(2), &live).unwrap();
        let MessageResponse::Channel([peer, transferred]) =
            hub.handle(1, MessageRequest::CreateChannel, &live).unwrap()
        else {
            panic!("channel")
        };
        let MessageResponse::Transferred(ports) = hub
            .handle(
                1,
                MessageRequest::Transfer {
                    ports: vec![transferred],
                },
                &live,
            )
            .unwrap()
        else {
            panic!("transfer")
        };
        hub.handle(
            1,
            post(WireValue {
                ports,
                hosts: vec![WireHost::Port(0)],
                ..payload()
            }),
            &live,
        )
        .unwrap();
        let failed_bytes = hub.retained_bytes();
        assert!(failed_bytes > parked_bytes);
        hub.handle(1, retire(1), &live).unwrap();
        assert_eq!(hub.root_document_identity(1), Some((2, 0)));
        assert_eq!(
            hub.retained_bytes(),
            failed_bytes,
            "stale retirement cannot discard active queued values"
        );
        hub.handle(1, retire(2), &live).unwrap();
        hub.handle(1, retire(2), &live).unwrap();
        assert_eq!(hub.root_document_identity(1), None);
        assert_eq!(hub.retained_bytes(), parked_bytes);
        assert!(matches!(
            hub.handle(1, MessageRequest::PollPort { port: peer, max: 1 }, &live)
                .unwrap(),
            MessageResponse::PortMessages {
                peer_closed: true,
                ..
            }
        ));
        hub.handle(1, register(3), &live).unwrap();
        hub.handle(
            1,
            post(WireValue {
                bytes: vec![9],
                ..payload()
            }),
            &live,
        )
        .unwrap();
        let newer_bytes = hub.retained_bytes();
        hub.handle(1, retire(2), &live).unwrap();
        assert_eq!(hub.root_document_identity(1), Some((3, 0)));
        assert_eq!(hub.retained_bytes(), newer_bytes);
        let MessageResponse::WindowMessages(messages) = hub
            .handle(
                1,
                MessageRequest::PollWindow {
                    target: WindowTarget::Root,
                    max: 1,
                },
                &live,
            )
            .unwrap()
        else {
            panic!("newer queue")
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload.bytes, vec![9]);
        hub.handle(1, retire(3), &live).unwrap();
        assert_eq!(hub.restore_document_authority(1, &parked).unwrap(), (1, 0));
        let MessageResponse::WindowMessages(messages) = hub
            .handle(
                1,
                MessageRequest::PollWindow {
                    target: WindowTarget::Root,
                    max: 1,
                },
                &live,
            )
            .unwrap()
        else {
            panic!("parked queue")
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload.bytes, vec![1]);
        assert_eq!(hub.retained_bytes(), 0);
    }

    #[test]
    fn root_worker_creator_epochs_cannot_be_resurrected_after_retirement() {
        let live = BTreeMap::from([(1, "https://root.test".into())]);
        let mut hub = MessageHub::default();
        let register = |generation, epoch| MessageRequest::RegisterRootDocument {
            document_generation: generation,
            stream_epoch: epoch,
        };
        let retire = |generation, epoch| MessageRequest::RetireRootDocument {
            document_generation: generation,
            stream_epoch: epoch,
        };
        hub.handle(1, register(3, 0), &live).unwrap();
        assert!(hub.root_document_authority(1, 3, 0).is_ok());
        hub.handle(1, register(3, 1), &live).unwrap();
        assert!(hub.root_document_authority(1, 3, 0).is_err());
        hub.handle(1, retire(3, 0), &live).unwrap();
        assert!(hub.root_document_authority(1, 3, 1).is_ok());
        hub.handle(1, retire(3, 1), &live).unwrap();
        assert!(hub.handle(1, register(3, 1), &live).is_err());
        hub.handle(1, register(4, 0), &live).unwrap();
        assert!(hub.root_document_authority(1, 4, 0).is_ok());
        assert!(
            hub.handle_agent(1, register(8, 0), "https://root.test", "root-key")
                .is_err()
        );
        hub.retire_page(1);
        assert!(hub.root_document_authority(1, 4, 0).is_err());
        assert!(hub.handle(1, register(5, 0), &BTreeMap::new()).is_err());
    }
    #[test]
    fn worker_frame_authority_requires_exact_document_and_preserves_opaque_identity() {
        let mut hub = MessageHub::default();
        let frame = "a".repeat(64);
        let live = BTreeMap::from([(1, "null".into()), (2, "null".into())]);
        let keys = BTreeMap::from([
            (1, "native-opaque-one".into()),
            (2, "native-opaque-two".into()),
        ]);
        hub.handle_with_origin_keys(
            1,
            MessageRequest::RegisterWindow {
                frame: frame.clone(),
                document_generation: 4,
                stream_epoch: 1,
                origin: "null".into(),
                inherit_origin: Some(WindowTarget::Root),
            },
            &live,
            &keys,
        )
        .unwrap();
        assert_eq!(
            hub.frame_document_authority(1, &frame, 4, 1).unwrap(),
            ("null".into(), "native-opaque-one".into())
        );
        assert!(hub.frame_document_authority(1, &frame, 4, 0).is_err());
        assert!(hub.frame_document_authority(2, &frame, 4, 1).is_err());
        hub.handle_with_origin_keys(
            1,
            MessageRequest::RegisterWindow {
                frame: frame.clone(),
                document_generation: 5,
                stream_epoch: 0,
                origin: "https://new.test".into(),
                inherit_origin: None,
            },
            &live,
            &keys,
        )
        .unwrap();
        assert!(hub.frame_document_authority(1, &frame, 4, 1).is_err());
        hub.handle_with_origin_keys(
            1,
            MessageRequest::RetireWindow {
                frame: frame.clone(),
                document_generation: 4,
                stream_epoch: 1,
            },
            &live,
            &keys,
        )
        .unwrap();
        assert_eq!(
            hub.frame_document_authority(1, &frame, 5, 0).unwrap().0,
            "https://new.test"
        );
        hub.handle_with_origin_keys(
            1,
            MessageRequest::RetireWindow {
                frame: frame.clone(),
                document_generation: 5,
                stream_epoch: 0,
            },
            &live,
            &keys,
        )
        .unwrap();
        assert!(hub.frame_document_authority(1, &frame, 5, 0).is_err());
    }
    #[test]
    fn broadcast_poll_preserves_shared_task_order_across_subscriptions() {
        let live = BTreeMap::from([(1, "https://a.test".into())]);
        let mut hub = MessageHub::default();
        hub.set_partition(Principal::Window(1), "site".into());
        let channels = (0..3)
            .map(|_| {
                let MessageResponse::BroadcastOpened(token) = hub
                    .handle(
                        1,
                        MessageRequest::BroadcastOpen {
                            target: WindowTarget::Root,
                            name: vec![65],
                        },
                        &live,
                    )
                    .unwrap()
                else {
                    panic!()
                };
                token
            })
            .collect::<Vec<_>>();
        for sender in [0, 2, 1] {
            let mut value = payload();
            value.bytes = vec![sender as u8];
            hub.handle(
                1,
                MessageRequest::BroadcastPost {
                    channel: channels[sender].clone(),
                    payload: value,
                },
                &live,
            )
            .unwrap();
        }
        let mut received = Vec::new();
        for _ in 0..3 {
            for (index, channel) in channels.iter().enumerate() {
                let MessageResponse::BroadcastMessages(messages) = hub
                    .handle(
                        1,
                        MessageRequest::BroadcastPoll {
                            channel: channel.clone(),
                            max: 32,
                        },
                        &live,
                    )
                    .unwrap()
                else {
                    panic!()
                };
                received.extend(
                    messages
                        .into_iter()
                        .map(|message| (index, message.payload.bytes[0])),
                );
            }
        }
        assert_eq!(
            received,
            vec![(1, 0), (2, 0), (0, 2), (1, 2), (0, 1), (2, 1)]
        );
    }
    #[test]
    fn broadcast_partitions_origin_names_close_and_source_exclusion() {
        let live = [
            (1, "https://a.test".into()),
            (2, "https://a.test".into()),
            (3, "https://a.test".into()),
            (4, "https://b.test".into()),
        ]
        .into();
        let mut hub = MessageHub::default();
        for page in [1, 2, 4] {
            hub.set_partition(Principal::Window(page), "site-a".into())
        }
        hub.set_partition(Principal::Window(3), "site-b".into());
        let mut channels = Vec::new();
        for page in 1..=4 {
            let MessageResponse::BroadcastOpened(token) = hub
                .handle(
                    page,
                    MessageRequest::BroadcastOpen {
                        target: WindowTarget::Root,
                        name: vec![0xd800],
                    },
                    &live,
                )
                .unwrap()
            else {
                panic!()
            };
            channels.push(token);
        }
        hub.handle(
            1,
            MessageRequest::BroadcastPost {
                channel: channels[0].clone(),
                payload: payload(),
            },
            &live,
        )
        .unwrap();
        for page in 1..=4 {
            let MessageResponse::BroadcastMessages(messages) = hub
                .handle(
                    page,
                    MessageRequest::BroadcastPoll {
                        channel: channels[(page - 1) as usize].clone(),
                        max: 32,
                    },
                    &live,
                )
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(messages.len(), usize::from(page == 2));
        }
        hub.handle(
            2,
            MessageRequest::BroadcastClose {
                channel: channels[1].clone(),
            },
            &live,
        )
        .unwrap();
        assert!(
            hub.handle(
                2,
                MessageRequest::BroadcastPost {
                    channel: channels[1].clone(),
                    payload: payload()
                },
                &live
            )
            .is_err()
        );
    }
    #[test]
    fn scoped_port_sends_reject_retired_realms_but_transferred_tokens_and_cleanup_survive() {
        let live = [(1, "https://a.test".into()), (2, "https://a.test".into())].into();
        let mut hub = MessageHub::default();
        let scoped = |generation, request| MessageRequest::InDocument {
            target: WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
            request: Box::new(request),
        };
        for (page, generation) in [(1, 1), (2, 4)] {
            hub.handle(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: generation,
                    stream_epoch: 0,
                },
                &live,
            )
            .unwrap();
        }
        let MessageResponse::Channel([a, b]) = hub
            .handle(1, scoped(1, MessageRequest::CreateChannel), &live)
            .unwrap()
        else {
            panic!()
        };
        let MessageResponse::Transferred(tokens) = hub
            .handle(
                1,
                scoped(1, MessageRequest::Transfer { ports: vec![b] }),
                &live,
            )
            .unwrap()
        else {
            panic!()
        };
        let mut transferred = payload();
        transferred.ports = tokens;
        hub.route_value(Principal::Window(1), Principal::Window(2), &mut transferred)
            .unwrap();
        hub.handle(
            1,
            MessageRequest::RetireRootDocument {
                document_generation: 1,
                stream_epoch: 0,
            },
            &live,
        )
        .unwrap();
        assert!(
            hub.handle(
                1,
                scoped(
                    1,
                    MessageRequest::PostPort {
                        port: a.clone(),
                        payload: payload()
                    }
                ),
                &live
            )
            .is_err()
        );
        let request = scoped(
            4,
            MessageRequest::PostPort {
                port: transferred.ports.remove(0),
                payload: payload(),
            },
        );
        assert!(request.wakes_recipients());
        hub.handle(2, request, &live).unwrap();
        hub.handle(1, MessageRequest::ClosePort { port: a }, &live)
            .unwrap();
    }
    #[test]
    fn agent_and_window_principals_do_not_alias_and_transferred_ports_are_remapped() {
        let live = [(1, "https://a.test".into())].into();
        let mut hub = MessageHub::default();
        let MessageResponse::Channel([a, b]) =
            hub.handle(1, MessageRequest::CreateChannel, &live).unwrap()
        else {
            panic!()
        };
        assert!(
            hub.handle_agent(
                1,
                MessageRequest::ClosePort { port: a.clone() },
                "https://a.test",
                "oryn:origin:https://a.test"
            )
            .is_err()
        );
        let MessageResponse::Transferred(ports) = hub
            .handle(1, MessageRequest::Transfer { ports: vec![b] }, &live)
            .unwrap()
        else {
            panic!()
        };
        let mut value = payload();
        value.ports = ports;
        hub.route_value(Principal::Window(1), Principal::Agent(1), &mut value)
            .unwrap();
        hub.handle(
            1,
            MessageRequest::PostPort {
                port: a,
                payload: payload(),
            },
            &live,
        )
        .unwrap();
        assert!(
            matches!(hub.handle_agent(1,MessageRequest::PollPort{port:value.ports.remove(0),max:1},"https://a.test","oryn:origin:https://a.test").unwrap(),MessageResponse::PortMessages{messages,..}if messages.len()==1)
        );
        assert!(
            hub.handle_agent(
                1,
                MessageRequest::PollWindow {
                    target: WindowTarget::Root,
                    max: 1
                },
                "https://a.test",
                "oryn:origin:https://a.test"
            )
            .is_err()
        );
    }
}

#[path = "message_bridge.rs"]
mod bridge;
pub use bridge::{CrossHubPortBridge, route_cross_hub_value};

#[cfg(test)]
mod scoped_window_sender_regressions {
    use super::*;
    #[test]
    fn scoped_window_status_requires_live_source_even_when_target_survives() {
        let mut hub = MessageHub::default();
        let live = BTreeMap::from([
            (1, "https://a.test".to_string()),
            (2, "https://b.test".to_string()),
        ]);
        for page in [1, 2] {
            hub.handle(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
                &live,
            )
            .unwrap();
        }
        let status = |generation, epoch, target| MessageRequest::InDocument {
            target,
            document_generation: generation,
            stream_epoch: epoch,
            request: Box::new(MessageRequest::WindowStatus {
                page: 2,
                target: WindowTarget::Root,
            }),
        };
        assert!(matches!(
            hub.handle(1, status(1, 0, WindowTarget::Root), &live)
                .unwrap(),
            MessageResponse::WindowLive(true)
        ));
        hub.handle(
            1,
            MessageRequest::RegisterRootDocument {
                document_generation: 2,
                stream_epoch: 0,
            },
            &live,
        )
        .unwrap();
        assert!(
            hub.handle(1, status(1, 0, WindowTarget::Root), &live)
                .is_err()
        );
        assert!(
            hub.handle(1, status(2, 1, WindowTarget::Root), &live)
                .is_err()
        );
        assert!(
            hub.handle(1, status(2, 0, WindowTarget::Frame("forged".into())), &live)
                .is_err()
        );
        assert!(matches!(
            hub.handle(1, status(2, 0, WindowTarget::Root), &live)
                .unwrap(),
            MessageResponse::WindowLive(true)
        ));
        let target_closed = BTreeMap::from([(1, "https://a.test".to_string())]);
        assert!(matches!(
            hub.handle(1, status(2, 0, WindowTarget::Root), &target_closed)
                .unwrap(),
            MessageResponse::WindowLive(false)
        ));
    }
    #[test]
    fn sender_generation_and_target_are_native_even_when_page_survives() {
        let mut hub = MessageHub::default();
        let live = BTreeMap::from([
            (1, "https://a.test".to_string()),
            (2, "https://a.test".to_string()),
        ]);
        for page in [1, 2] {
            hub.handle(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
                &live,
            )
            .unwrap();
        }
        let send = |generation, source| MessageRequest::InDocument {
            target: WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
            request: Box::new(MessageRequest::PostWindow {
                target_page: 2,
                target: WindowTarget::Root,
                source,
                target_origin: TargetOrigin::Any,
                payload: WireValue {
                    version: 1,
                    bytes: vec![0],
                    hosts: vec![],
                    buffers: vec![],
                    ports: vec![],
                    shared: vec![],
                    shared_queue: None,
                    shared_recipient: None,
                    wasm_modules: vec![],
                },
            }),
        };
        hub.handle(1, send(1, WindowTarget::Root), &live).unwrap();
        hub.handle(
            1,
            MessageRequest::RegisterRootDocument {
                document_generation: 2,
                stream_epoch: 0,
            },
            &live,
        )
        .unwrap();
        assert!(hub.handle(1, send(1, WindowTarget::Root), &live).is_err());
        hub.handle(1, send(2, WindowTarget::Root), &live).unwrap();
        assert!(
            hub.handle(1, send(2, WindowTarget::Frame("forged".into())), &live)
                .is_err()
        );
    }
}

#[cfg(test)]
mod broadcast_document_custody_regressions {
    use super::*;
    fn scoped(generation: u64, request: MessageRequest) -> MessageRequest {
        MessageRequest::InDocument {
            target: WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
            request: Box::new(request),
        }
    }
    fn payload() -> WireValue {
        WireValue {
            version: 1,
            bytes: vec![0],
            hosts: vec![],
            buffers: vec![],
            ports: vec![],
            shared: vec![],
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: vec![],
        }
    }
    #[test]
    fn parked_subscriptions_are_ineligible_and_exact_retirement_reclaims_lost_close() {
        let mut hub = MessageHub::default();
        let live = BTreeMap::from([
            (1, "https://a.test".to_string()),
            (2, "https://a.test".to_string()),
        ]);
        let mut tokens = Vec::new();
        for page in [1, 2] {
            hub.set_partition(Principal::Window(page), "same".into());
            hub.handle(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
                &live,
            )
            .unwrap();
            let MessageResponse::BroadcastOpened(token) = hub
                .handle(
                    page,
                    scoped(
                        1,
                        MessageRequest::BroadcastOpen {
                            target: WindowTarget::Root,
                            name: vec![7],
                        },
                    ),
                    &live,
                )
                .unwrap()
            else {
                panic!("subscription")
            };
            tokens.push(token);
        }
        assert!(
            hub.handle(
                1,
                MessageRequest::BroadcastPoll {
                    channel: tokens[0].clone(),
                    max: 1
                },
                &live
            )
            .is_err(),
            "scoped subscription cannot fall back to page identity"
        );
        let parked = hub.park_document_authority(1, 1, 0).unwrap();
        hub.handle(
            2,
            scoped(
                1,
                MessageRequest::BroadcastPost {
                    channel: tokens[1].clone(),
                    payload: payload(),
                },
            ),
            &live,
        )
        .unwrap();
        assert!(hub.broadcasts[&tokens[0].id].queue.is_empty());
        hub.restore_document_authority(1, &parked).unwrap();
        hub.handle(
            2,
            scoped(
                1,
                MessageRequest::BroadcastPost {
                    channel: tokens[1].clone(),
                    payload: payload(),
                },
            ),
            &live,
        )
        .unwrap();
        assert_eq!(hub.broadcasts[&tokens[0].id].queue.len(), 1);
        hub.handle(
            1,
            MessageRequest::RegisterRootDocument {
                document_generation: 2,
                stream_epoch: 0,
            },
            &live,
        )
        .unwrap();
        assert!(!hub.broadcasts.contains_key(&tokens[0].id));
        assert!(hub.broadcasts.contains_key(&tokens[1].id));
        let MessageResponse::BroadcastOpened(new) = hub
            .handle(
                1,
                scoped(
                    2,
                    MessageRequest::BroadcastOpen {
                        target: WindowTarget::Root,
                        name: vec![7],
                    },
                ),
                &live,
            )
            .unwrap()
        else {
            panic!("replacement")
        };
        hub.handle(
            1,
            MessageRequest::RetireRootDocument {
                document_generation: 1,
                stream_epoch: 0,
            },
            &live,
        )
        .unwrap();
        assert!(hub.broadcasts.contains_key(&new.id));
        let parked = hub.park_document_authority(1, 2, 0).unwrap();
        hub.discard_parked_document(1, &parked).unwrap();
        assert!(!hub.broadcasts.contains_key(&new.id));
        assert!(hub.broadcasts.contains_key(&tokens[1].id));
    }
}

#[cfg(test)]
mod shared_disposal_tests {
    use super::*;
    use crate::shared_memory::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    struct Backend {
        fail: AtomicBool,
        live: Mutex<BTreeSet<SharedMemoryHandoff>>,
    }
    impl SharedMessageBackend for Backend {
        fn adopt(
            &self,
            _: Principal,
            _: &[SharedMemoryReference],
        ) -> Result<Arc<dyn SharedMessageLease>, String> {
            Err("unused".into())
        }
        fn discard_delivered(
            &self,
            target: Principal,
            references: &[SharedMemoryReference],
        ) -> Result<(), String> {
            assert_eq!(target, Principal::Window(7));
            for reference in references {
                if reference.handoff.0[0] == 1 && self.fail.load(Ordering::SeqCst) {
                    return Err("injected shared cleanup failure".into());
                }
                self.live.lock().unwrap().remove(&reference.handoff);
            }
            Ok(())
        }
        fn discard_offered(&self, _: Principal, _: &[SharedMemoryReference]) -> Result<(), String> {
            Err("unused".into())
        }
        fn retire_principal(&self, _: Principal) {}
    }
    fn reference(id: u8) -> SharedMemoryReference {
        SharedMemoryReference {
            handoff: SharedMemoryHandoff([id; 32]),
            descriptor: SharedMemoryDescriptor {
                token: SharedMemoryToken([9; 32]),
                byte_length: 8,
                mapped_length: 4096,
                maximum_length: 8,
                data_offset: 0,
                reserved_length: 4096,
                guard_length: 0,
                wasm: false,
                memory64: false,
                growable: false,
            },
        }
    }
    #[test]
    fn mixed_disposal_releases_ports_and_independent_claims_while_retaining_exact_failure() {
        let backend = Arc::new(Backend {
            fail: AtomicBool::new(true),
            live: Mutex::new(BTreeSet::from([reference(1).handoff, reference(2).handoff])),
        });
        let mut hub = MessageHub::default();
        hub.set_shared_message_backend(backend.clone());
        let [port, _] = hub
            .create_entangled(Principal::Window(7), Principal::Agent(3))
            .unwrap();
        let wire = WireValue {
            shared: vec![reference(1), reference(2)],
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: Vec::new(),
            hosts: Vec::new(),
            buffers: Vec::new(),
            ports: vec![port.clone()],
        };
        assert!(
            hub.dispose_queued_value(Principal::Window(7), wire.clone())
                .is_err()
        );
        assert!(hub.endpoint(Principal::Window(7), &port, false).is_err());
        assert_eq!(backend.live.lock().unwrap().len(), 1);
        assert_eq!(hub.pending_shared_disposals.len(), 1);
        assert!(
            hub.dispose_queued_value(Principal::Window(7), wire)
                .is_err()
        );
        assert_eq!(
            hub.pending_shared_disposals.len(),
            1,
            "retry retains one exact handoff debt"
        );
        let replacement = Arc::new(Backend {
            fail: AtomicBool::new(false),
            live: Mutex::new(BTreeSet::from([reference(1).handoff])),
        });
        hub.set_shared_message_backend(replacement.clone());
        backend.fail.store(false, Ordering::SeqCst);
        hub.retry_shared_disposals().unwrap();
        assert!(backend.live.lock().unwrap().is_empty());
        assert_eq!(
            replacement.live.lock().unwrap().len(),
            1,
            "old debt belongs to its original backend"
        );
        assert!(hub.pending_shared_disposals.is_empty());
        hub.retry_shared_disposals().unwrap();
    }
}

#[cfg(test)]
mod native_pixel_wire_regressions {
    use super::*;
    fn value(host: WireHost) -> WireValue {
        WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: Vec::new(),
            hosts: vec![host],
            buffers: Vec::new(),
            ports: Vec::new(),
        }
    }
    #[test]
    fn audio_pcm_shape_and_finite_rate_are_validated() {
        let data = AudioDataRecord {
            format: "f32".into(),
            sample_rate: 44100.,
            frames: 4,
            channels: 1,
            timestamp: 0,
            bytes: vec![0; 16],
        };
        assert!(value(WireHost::AudioData(data.clone())).validate().is_ok());
        let mut truncated = data.clone();
        truncated.bytes.pop();
        assert!(value(WireHost::AudioData(truncated)).validate().is_err());
        let mut invalid = data;
        invalid.sample_rate = f32::NAN;
        assert!(value(WireHost::AudioData(invalid)).validate().is_err());
    }
    #[test]
    fn pixel_dimensions_and_frame_crop_are_validated_before_allocation() {
        assert!(
            value(WireHost::ImageBitmap {
                width: 2,
                height: 1,
                rgba: vec![0; 8]
            })
            .validate()
            .is_ok()
        );
        assert!(
            value(WireHost::ImageBitmap {
                width: 2,
                height: 1,
                rgba: vec![0; 4]
            })
            .validate()
            .is_err()
        );
        assert!(
            value(WireHost::OffscreenCanvas {
                width: 0,
                height: 5,
                rgba: Vec::new(),
                origin_clean: true
            })
            .validate()
            .is_ok()
        );
        let frame = VideoFrameData {
            width: 2,
            height: 2,
            rgba: vec![0; 16],
            planes: None,
            timestamp: 17,
            duration: Some(4),
            visible: [0, 0, 2, 2],
            display: [2, 2],
            rotation: 0,
            flip: false,
        };
        let mut planar = frame.clone();
        planar.planes = Some(vec![0; 6]);
        assert!(
            value(WireHost::VideoFrame(planar.clone()))
                .validate()
                .is_ok()
        );
        planar.planes.as_mut().unwrap().pop();
        assert!(value(WireHost::VideoFrame(planar)).validate().is_err());
        assert!(
            value(WireHost::VideoFrame(frame.clone()))
                .validate()
                .is_ok()
        );
        let mut outside = frame.clone();
        outside.visible = [1, 1, 2, 2];
        assert!(value(WireHost::VideoFrame(outside)).validate().is_err());
        let mut angle = frame;
        angle.rotation = 91;
        assert!(value(WireHost::VideoFrame(angle)).validate().is_err());
    }
}

#[cfg(test)]
mod pending_port_transfer_regressions {
    use super::*;
    fn value() -> WireValue {
        WireValue {
            shared: vec![],
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: vec![],
            version: 1,
            bytes: vec![7],
            hosts: vec![],
            buffers: vec![],
            ports: vec![],
        }
    }
    #[test]
    fn readiness_does_not_consume_or_reassign_queued_transfers() {
        let mut hub = MessageHub::default();
        let pages = BTreeMap::from([(1, "https://a.test".into())]);
        let MessageResponse::Channel([sender, receiver]) = hub
            .handle(1, MessageRequest::CreateChannel, &pages)
            .unwrap()
        else {
            panic!()
        };
        assert!(matches!(
            hub.handle(
                1,
                MessageRequest::PortPending {
                    port: receiver.clone()
                },
                &pages
            )
            .unwrap(),
            MessageResponse::PortPending(false)
        ));
        hub.handle(
            1,
            MessageRequest::PostPort {
                port: sender,
                payload: value(),
            },
            &pages,
        )
        .unwrap();
        let bytes = hub.retained_bytes();
        for _ in 0..2 {
            assert!(matches!(
                hub.handle(
                    1,
                    MessageRequest::PortPending {
                        port: receiver.clone()
                    },
                    &pages
                )
                .unwrap(),
                MessageResponse::PortPending(true)
            ));
            assert_eq!(hub.retained_bytes(), bytes);
        }
        let MessageResponse::Transferred(mut tokens) = hub
            .handle(
                1,
                MessageRequest::Transfer {
                    ports: vec![receiver.clone()],
                },
                &pages,
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(
            hub.handle(1, MessageRequest::PortPending { port: receiver }, &pages)
                .is_err()
        );
        let token = tokens.pop().unwrap();
        assert_eq!(hub.ports[&token.id].queue.len(), 1);
        assert_eq!(hub.retained_bytes(), bytes);
    }
    #[test]
    fn transferred_buffer_metadata_rejects_ambiguous_indices() {
        let mut wire = value();
        wire.buffers = vec![vec![]];
        wire.hosts = vec![WireHost::TransferredArrayBuffer {
            index: 0,
            bytes: vec![1, 2, 3],
        }];
        wire.validate().unwrap();
        let retained = wire.retained_bytes();
        wire.hosts.push(WireHost::TransferredArrayBuffer {
            index: 0,
            bytes: vec![4],
        });
        assert!(wire.validate().is_err());
        assert!(wire.retained_bytes() > retained);
        wire.hosts.pop();
        wire.buffers[0].push(5);
        assert!(wire.validate().is_err());
        wire.buffers[0].clear();
        wire.hosts[0] = WireHost::TransferredArrayBuffer {
            index: 1,
            bytes: vec![1],
        };
        assert!(wire.validate().is_err());
    }
}

#[cfg(test)]
mod opaque_broadcast_regressions {
    use super::*;
    #[test]
    fn opaque_channels_use_authority_and_partition_without_rejecting_creation() {
        let mut hub = MessageHub::default();
        let live = BTreeMap::from([
            (1, "null".into()),
            (2, "null".into()),
            (3, "null".into()),
            (4, "null".into()),
        ]);
        let keys = BTreeMap::from([
            (1, "opaque-a".into()),
            (2, "opaque-a".into()),
            (3, "opaque-b".into()),
            (4, "opaque-a".into()),
        ]);
        for page in 1..=4 {
            hub.set_partition(
                Principal::Window(page),
                if page == 4 { "other" } else { "site" }.into(),
            );
        }
        hub.set_partition(Principal::Agent(5), "site".into());
        let mut channels = Vec::new();
        for page in 1..=4 {
            let MessageResponse::BroadcastOpened(token) = hub
                .handle_with_origin_keys(
                    page,
                    MessageRequest::BroadcastOpen {
                        target: WindowTarget::Root,
                        name: vec![65],
                    },
                    &live,
                    &keys,
                )
                .unwrap()
            else {
                panic!()
            };
            channels.push(token);
        }
        let MessageResponse::BroadcastOpened(worker) = hub
            .handle_agent(
                5,
                MessageRequest::BroadcastOpen {
                    target: WindowTarget::Root,
                    name: vec![65],
                },
                "null",
                "opaque-a",
            )
            .unwrap()
        else {
            panic!()
        };
        let payload = WireValue {
            shared: vec![],
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: vec![],
            version: 1,
            bytes: vec![9],
            hosts: vec![],
            buffers: vec![],
            ports: vec![],
        };
        hub.handle_with_origin_keys(
            1,
            MessageRequest::BroadcastPost {
                channel: channels[0].clone(),
                payload,
            },
            &live,
            &keys,
        )
        .unwrap();
        for page in 1..=4 {
            let MessageResponse::BroadcastMessages(messages) = hub
                .handle_with_origin_keys(
                    page,
                    MessageRequest::BroadcastPoll {
                        channel: channels[page as usize - 1].clone(),
                        max: 1,
                    },
                    &live,
                    &keys,
                )
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(messages.len(), usize::from(page == 2));
            if let Some(message) = messages.first() {
                assert_eq!(message.origin, "null");
            }
        }
        let MessageResponse::BroadcastMessages(messages) = hub
            .handle_agent(
                5,
                MessageRequest::BroadcastPoll {
                    channel: worker,
                    max: 1,
                },
                "null",
                "opaque-a",
            )
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].origin, "null");
    }
}
