//! Native CacheStorage protocol; creator authority is supplied outside these records.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheResponse {
    #[serde(default)]
    pub timing: Option<crate::platform::ResourceTimingMetadata>,
    #[serde(default)]
    pub internal_response: Option<crate::platform::InternalResponseMetadata>,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub has_body: bool,
    pub redirected: bool,
    pub url: String,
    pub response_type: String,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct CacheQuery {
    pub ignore_search: bool,
    pub ignore_method: bool,
    pub ignore_vary: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CacheOperation {
    ReleaseHandle {
        cache: u64,
    },
    Open(Vec<u16>),
    Has(Vec<u16>),
    DeleteCache(Vec<u16>),
    CacheNames,
    Match {
        cache: Option<u64>,
        request: CacheRequest,
        query: CacheQuery,
    },
    MatchNamed {
        name: Vec<u16>,
        request: CacheRequest,
        query: CacheQuery,
    },
    MatchAll {
        cache: u64,
        request: Option<CacheRequest>,
        query: CacheQuery,
    },
    Keys {
        cache: u64,
        request: Option<CacheRequest>,
        query: CacheQuery,
    },
    Delete {
        cache: u64,
        request: CacheRequest,
        query: CacheQuery,
    },
    PutAll {
        cache: u64,
        entries: Vec<(CacheRequest, CacheResponse)>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CacheResult {
    Cache(u64),
    Bool(bool),
    Names(Vec<Vec<u16>>),
    Responses(Vec<CacheResponse>),
    Requests(Vec<CacheRequest>),
    Done,
}
