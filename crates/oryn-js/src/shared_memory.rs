//! Native agent-cluster memory capabilities. These types carry identity only;
//! shared bytes travel through an OS descriptor, never a serialized byte copy.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SharedMemoryToken(pub [u8; 32]);
impl std::fmt::Debug for SharedMemoryToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedMemoryToken([redacted])")
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct SharedMemoryDescriptor {
    pub token: SharedMemoryToken,
    pub byte_length: u64,
    pub mapped_length: u64,
    pub maximum_length: u64,
    pub data_offset: u64,
    pub reserved_length: u64,
    pub guard_length: u64,
    pub wasm: bool,
    pub memory64: bool,
    pub growable: bool,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SharedMemoryHandoff(pub [u8; 32]);
impl std::fmt::Debug for SharedMemoryHandoff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedMemoryHandoff([redacted])")
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SharedMemoryReference {
    pub descriptor: SharedMemoryDescriptor,
    pub handoff: SharedMemoryHandoff,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SharedMemoryRecipient {
    Window {
        page: u64,
        target: crate::message_wire::WindowTarget,
        document_generation: u64,
        stream_epoch: u64,
    },
    Agent(u64),
}
impl SharedMemoryRecipient {
    pub fn principal(&self) -> crate::message_wire::Principal {
        match self {
            Self::Window { page, .. } => crate::message_wire::Principal::Window(*page),
            Self::Agent(id) => crate::message_wire::Principal::Agent(*id),
        }
    }
}
/// Parent-only queue ownership; implementations retain actual backing leases.
/// This object never crosses serialization and cannot be supplied by scripts.
pub trait SharedMessageLease: std::fmt::Debug + Send + Sync {
    fn deliver(&self, target: SharedMemoryRecipient) -> Result<Vec<SharedMemoryReference>, String>;
}
pub trait SharedMessageBackend: Send + Sync {
    fn adopt(
        &self,
        source: crate::message_wire::Principal,
        references: &[SharedMemoryReference],
    ) -> Result<std::sync::Arc<dyn SharedMessageLease>, String>;
    fn discard_delivered(
        &self,
        target: crate::message_wire::Principal,
        references: &[SharedMemoryReference],
    ) -> Result<(), String>;
    fn discard_offered(
        &self,
        source: crate::message_wire::Principal,
        references: &[SharedMemoryReference],
    ) -> Result<(), String>;
    fn retire_principal(&self, source: crate::message_wire::Principal);
}
/// Native client-issued lane sequence permits compact replay tombstones after
/// acknowledged cleanup without retaining one record per historical wait.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WaitAdmissionId {
    pub lane: [u8; 32],
    pub sequence: u64,
}
impl std::fmt::Debug for WaitAdmissionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WaitAdmissionId")
            .field("sequence", &self.sequence)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum SharedMemoryRequest {
    WaitAdmission {
        id: WaitAdmissionId,
        descriptor: SharedMemoryDescriptor,
        acquire: bool,
        offset: u64,
        width64: bool,
        expected: u64,
        timeout_ns: Option<u64>,
    },
    CancelAdmission {
        id: WaitAdmissionId,
    },
    Bind {
        parent: Option<crate::worker_wire::Creator>,
    },
    Status,
    Offer {
        descriptor: SharedMemoryDescriptor,
    },
    CommitOffers {
        references: Vec<SharedMemoryReference>,
    },
    AbortOffer {
        reference: SharedMemoryReference,
    },
    OpenDelivery {
        reference: SharedMemoryReference,
    },
    ReleaseDelivery {
        reference: SharedMemoryReference,
    },
    Allocate {
        byte_length: u64,
        maximum_length: Option<u64>,
        granularity: u64,
        guard_length: u64,
        wasm: bool,
        memory64: bool,
    },
    Open {
        descriptor: SharedMemoryDescriptor,
    },
    Release {
        token: SharedMemoryToken,
    },
    GrowAbsolute {
        token: SharedMemoryToken,
        length: u64,
    },
    GrowByPages {
        token: SharedMemoryToken,
        pages: u64,
        maximum_pages: u64,
    },
    WaitEnqueue {
        token: SharedMemoryToken,
        offset: u64,
        width64: bool,
        expected: u64,
        timeout_ns: Option<u64>,
    },
    WaitPoll {
        id: SharedWaitId,
    },
    WaitCancel {
        id: SharedWaitId,
    },
    Notify {
        token: SharedMemoryToken,
        offset: u64,
        count: u64,
    },
    CloneOrigin,
}
impl SharedMemoryRequest {
    /// These operations carry an already-issued unguessable capability and
    /// preserve a physical mapping's lifetime beyond its creating document.
    /// The broker still validates the exact route, holder and token on each call.
    pub fn requires_current_document(&self) -> bool {
        !matches!(
            self,
            Self::CancelAdmission { .. }
                | Self::Release { .. }
                | Self::GrowAbsolute { .. }
                | Self::GrowByPages { .. }
                | Self::WaitEnqueue { .. }
                | Self::WaitPoll { .. }
                | Self::WaitCancel { .. }
                | Self::Notify { .. }
                | Self::AbortOffer { .. }
                | Self::ReleaseDelivery { .. }
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SharedMemoryResponse {
    pub isolated: bool,
    #[serde(default)]
    pub outcome: SharedMemoryOutcome,
    pub descriptor: Option<SharedMemoryDescriptor>,
    #[cfg(unix)]
    #[serde(skip)]
    pub native_descriptor: Option<std::sync::Arc<std::os::fd::OwnedFd>>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SharedWaitId(pub [u8; 32]);
impl std::fmt::Debug for SharedWaitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SharedWaitId([redacted])")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SharedWaitResult {
    Pending(SharedWaitId),
    Ok,
    NotEqual,
    TimedOut,
    Cancelled,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub enum SharedMemoryOutcome {
    #[default]
    None,
    Reference(SharedMemoryReference),
    Wait(SharedWaitResult),
    Length(u64),
    Count(u64),
    CloneOrigin([u8; 32]),
}
