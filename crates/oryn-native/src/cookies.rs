//! Parent-owned RFC6265bis cookie selection.
//!
//! The transport deliberately knows nothing about browser cookie semantics.
//! Every request supplies an unforgeable context and this store decides both
//! cookie creation and delivery before the request reaches reqwest.

use std::{
    collections::BTreeMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use cookie::{Cookie, SameSite};
use redb::{Database, ReadableDatabase, TableDefinition};
use serde::{Deserialize, Serialize};
use url::Url;

const PERSISTENT_COOKIES_V1: TableDefinition<&str, &[u8]> =
    TableDefinition::new("persistent_cookies_v1");
const PERSISTENT_COOKIES_V2: TableDefinition<&str, &[u8]> =
    TableDefinition::new("persistent_cookies_v2");
pub const SAME_SITE_POLICY_VERSION: &str = "rfc6265bis-22-chromium-lax-unsafe-120s";
pub const PSL_IMPLEMENTATION: &str = "psl-crate-2";
const LAX_ALLOW_UNSAFE_MS: u64 = 120_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SameSiteProfile {
    #[default]
    ModernChromium,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CookieCredentials {
    Omit,
    SameOrigin,
    #[default]
    Include,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CookieRequestKind {
    Document,
    Frame,
    Form,
    Script,
    Style,
    Image,
    Font,
    Media,
    Object,
    Manifest,
    Worker,
    Fetch,
    Xhr,
    WebSocket,
    #[default]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieRequestContext {
    #[serde(default)]
    pub fetch_metadata: crate::fetch_metadata::Metadata,
    #[serde(skip)]
    pub dns_timing: Option<oryn_net::DnsTiming>,
    #[serde(default)]
    pub fetch_cache: oryn_js::platform::FetchCacheMode,
    #[serde(default)]
    pub navigation: oryn_js::platform::NavigationRequestFlags,
    #[serde(default)]
    pub credentials: CookieCredentials,
    #[serde(default)]
    pub kind: CookieRequestKind,
    #[serde(default)]
    pub initiator_url: Option<String>,
    #[serde(default)]
    pub top_level_url: Option<String>,
    #[serde(default)]
    pub ancestor_urls: Vec<String>,
    #[serde(default)]
    pub top_level_navigation: bool,
    #[serde(default)]
    pub redirect_chain_cross_site: bool,
}

impl Default for CookieRequestContext {
    fn default() -> Self {
        Self {
            fetch_metadata: Default::default(),
            dns_timing: None,
            fetch_cache: Default::default(),
            navigation: Default::default(),
            credentials: CookieCredentials::Include,
            kind: CookieRequestKind::Other,
            initiator_url: None,
            top_level_url: None,
            ancestor_urls: Vec::new(),
            top_level_navigation: false,
            redirect_chain_cross_site: false,
        }
    }
}

impl CookieRequestContext {
    pub fn legacy(credentials: bool) -> Self {
        Self {
            credentials: if credentials {
                CookieCredentials::Include
            } else {
                CookieCredentials::Omit
            },
            ..Self::default()
        }
    }

    pub fn navigation(top_level_url: Option<String>) -> Self {
        Self {
            kind: CookieRequestKind::Document,
            top_level_url,
            top_level_navigation: true,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemefulSite {
    pub scheme: String,
    pub registrable_domain: String,
}

impl SchemefulSite {
    pub fn from_url(url: &Url) -> Option<Self> {
        let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
        let registrable_domain = if host.parse::<IpAddr>().is_ok() || host == "localhost" {
            host
        } else {
            let bytes = psl::domain_str(&host)?;
            bytes.to_ascii_lowercase()
        };
        Some(Self {
            scheme: url.scheme().to_ascii_lowercase(),
            registrable_domain,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredSameSite {
    Default,
    Lax,
    Strict,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredCookie {
    #[serde(default)]
    partition_key: Option<String>,
    name: String,
    value: String,
    domain: String,
    host_only: bool,
    path: String,
    secure: bool,
    http_only: bool,
    same_site: StoredSameSite,
    created_at_ms: Option<u64>,
    #[serde(default)]
    last_access_ms: u64,
    expires_at_ms: Option<u64>,
}

impl StoredCookie {
    fn key(&self) -> (Option<String>, String, String, String) {
        (
            self.partition_key.clone(),
            self.domain.clone(),
            self.path.clone(),
            self.name.clone(),
        )
    }

    fn expired(&self, now_ms: u64) -> bool {
        self.expires_at_ms.is_some_and(|expiry| expiry <= now_ms)
    }

    fn domain_matches(&self, host: &str) -> bool {
        host == self.domain
            || (!self.host_only
                && host
                    .strip_suffix(&self.domain)
                    .is_some_and(|prefix| prefix.ends_with('.')))
    }

    fn path_matches(&self, path: &str) -> bool {
        path == self.path
            || path
                .strip_prefix(&self.path)
                .is_some_and(|remaining| self.path.ends_with('/') || remaining.starts_with('/'))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieDecision {
    pub included: usize,
    pub withheld: usize,
    pub reasons: BTreeMap<String, usize>,
}

impl CookieDecision {
    fn withhold(&mut self, reason: &str) {
        self.withheld += 1;
        *self.reasons.entry(reason.into()).or_default() += 1;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CookieStoreError {
    #[error("invalid Set-Cookie value: {0}")]
    Invalid(String),
    #[error("persistent cookie store failed: {0}")]
    Persistent(String),
}

/// Browser policy is independent of SameSite and CHIPS. Grants are scoped to
/// the browser context and expire; persistent cookie data never persists consent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThirdPartyCookiePolicy {
    #[default]
    Block,
    Allow,
}
#[derive(Default)]
struct StorageAccessPolicy {
    mode: ThirdPartyCookiePolicy,
    grants: BTreeMap<(String, String), u64>,
}
pub struct ContextCookieStore {
    access: Mutex<StorageAccessPolicy>,
    cookies: Arc<Mutex<BTreeMap<(Option<String>, String, String, String), StoredCookie>>>,
    database: Option<Arc<Database>>,
    now: Arc<dyn Fn() -> u64 + Send + Sync>,
    #[cfg(test)]
    fail_next_commit: std::sync::atomic::AtomicBool,
}

impl ContextCookieStore {
    pub fn memory() -> Arc<Self> {
        Arc::new(Self {
            access: Mutex::new(StorageAccessPolicy::default()),
            cookies: Arc::new(Mutex::new(BTreeMap::new())),
            database: None,
            now: Arc::new(now_ms),
            #[cfg(test)]
            fail_next_commit: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn persistent(database: Arc<Database>) -> Result<Arc<Self>, CookieStoreError> {
        let store = Arc::new(Self {
            access: Mutex::new(StorageAccessPolicy::default()),
            cookies: Arc::new(Mutex::new(BTreeMap::new())),
            database: Some(database),
            now: Arc::new(now_ms),
            #[cfg(test)]
            fail_next_commit: std::sync::atomic::AtomicBool::new(false),
        });
        store.load()?;
        Ok(store)
    }

    #[cfg(test)]
    fn memory_with_clock(now: Arc<dyn Fn() -> u64 + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            access: Mutex::new(StorageAccessPolicy::default()),
            cookies: Arc::new(Mutex::new(BTreeMap::new())),
            database: None,
            now,
            fail_next_commit: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn for_context(&self, mode: ThirdPartyCookiePolicy) -> Arc<Self> {
        Arc::new(Self {
            access: Mutex::new(StorageAccessPolicy {
                mode,
                grants: BTreeMap::new(),
            }),
            cookies: self.cookies.clone(),
            database: self.database.clone(),
            now: self.now.clone(),
            #[cfg(test)]
            fail_next_commit: std::sync::atomic::AtomicBool::new(false),
        })
    }
    pub fn set_third_party_policy(&self, mode: ThirdPartyCookiePolicy) {
        if let Ok(mut access) = self.access.lock() {
            access.mode = mode;
            access.grants.clear();
        }
    }
    pub fn grant_storage_access(&self, top: &Url, embedded: &Url) -> Result<(), CookieStoreError> {
        if !is_potentially_trustworthy(top) || !is_potentially_trustworthy(embedded) {
            return Err(CookieStoreError::Invalid(
                "storage access requires secure principals".into(),
            ));
        }
        let top = SchemefulSite::from_url(top).ok_or_else(|| {
            CookieStoreError::Invalid("opaque top-level storage principal".into())
        })?;
        if embedded.origin().is_tuple() == false {
            return Err(CookieStoreError::Invalid(
                "opaque embedded storage principal".into(),
            ));
        }
        let key = (
            format!("{}://{}", top.scheme, top.registrable_domain),
            embedded.origin().ascii_serialization(),
        );
        let now = (self.now)();
        let mut access = self
            .access
            .lock()
            .map_err(|_| CookieStoreError::Persistent("storage access policy poisoned".into()))?;
        access.grants.retain(|_, expiry| *expiry > now);
        if access.grants.len() >= 1024 && !access.grants.contains_key(&key) {
            return Err(CookieStoreError::Invalid(
                "storage access grant capacity".into(),
            ));
        }
        access
            .grants
            .insert(key, now.saturating_add(30 * 24 * 60 * 60 * 1000));
        Ok(())
    }
    pub fn has_storage_access(&self, url: &Url, context: &CookieRequestContext) -> bool {
        if self
            .access
            .lock()
            .is_ok_and(|access| access.mode == ThirdPartyCookiePolicy::Allow)
        {
            return true;
        }
        if context.top_level_navigation {
            return true;
        }
        let Some(top) = context
            .top_level_url
            .as_deref()
            .and_then(|url| Url::parse(url).ok())
            .and_then(|url| SchemefulSite::from_url(&url))
        else {
            return false;
        };
        let target = SchemefulSite::from_url(url);
        if target.as_ref() == Some(&top)
            && context.ancestor_urls.iter().all(|ancestor| {
                Url::parse(ancestor)
                    .ok()
                    .and_then(|url| SchemefulSite::from_url(&url))
                    .as_ref()
                    == Some(&top)
            })
        {
            return true;
        }
        let Ok(access) = self.access.lock() else {
            return false;
        };
        access.mode == ThirdPartyCookiePolicy::Allow
            || access
                .grants
                .get(&(
                    format!("{}://{}", top.scheme, top.registrable_domain),
                    url.origin().ascii_serialization(),
                ))
                .is_some_and(|expiry| *expiry > (self.now)())
    }

    pub fn header(
        &self,
        request_url: &Url,
        context: &CookieRequestContext,
    ) -> (Option<String>, CookieDecision) {
        self.header_for_method(request_url, "GET", context)
    }

    pub fn header_for_method(
        &self,
        request_url: &Url,
        method: &str,
        context: &CookieRequestContext,
    ) -> (Option<String>, CookieDecision) {
        let mut decision = CookieDecision {
            included: 0,
            withheld: 0,
            reasons: BTreeMap::new(),
        };
        if matches!(context.credentials, CookieCredentials::Omit) {
            decision.reasons.insert("credentials_omit".into(), 1);
            return (None, decision);
        }
        if matches!(context.credentials, CookieCredentials::SameOrigin)
            && context
                .initiator_url
                .as_deref()
                .and_then(|url| Url::parse(url).ok())
                .is_some_and(|source| source.origin() != request_url.origin())
        {
            decision
                .reasons
                .insert("credentials_cross_origin".into(), 1);
            return (None, decision);
        }
        let Some(host) = request_url.host_str().map(|host| host.to_ascii_lowercase()) else {
            decision.reasons.insert("opaque_target".into(), 1);
            return (None, decision);
        };
        let unpartitioned = self.has_storage_access(request_url, context);
        let same_site = request_is_same_site(request_url, context);
        let safe_method = is_safe_method(method);
        let now = (self.now)();
        let mut selected = Vec::new();
        if let Ok(mut cookies) = self.cookies.lock() {
            for cookie in cookies.values_mut() {
                let reason = if cookie.expired(now) {
                    Some("expired")
                } else if cookie.partition_key.is_none() && !unpartitioned {
                    Some("third_party_blocked")
                } else if cookie.partition_key.is_some()
                    && cookie.partition_key != partition_key(request_url, context)
                {
                    Some("partition_mismatch")
                } else if !cookie.domain_matches(&host) || !cookie.path_matches(request_url.path())
                {
                    Some("scope_mismatch")
                } else if cookie.secure && !is_potentially_trustworthy(request_url) {
                    Some("secure_transport_required")
                } else if !same_site
                    && !same_site_allows_cross_site(cookie, context, safe_method, now)
                {
                    Some("same_site")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    decision.withhold(reason);
                } else {
                    decision.included += 1;
                    cookie.last_access_ms = now;
                    selected.push(cookie.clone());
                }
            }
        }
        selected.sort_by(|left, right| {
            right
                .path
                .len()
                .cmp(&left.path.len())
                .then(left.created_at_ms.cmp(&right.created_at_ms))
        });
        let header = (!selected.is_empty()).then(|| {
            selected
                .into_iter()
                .map(|cookie| format!("{}={}", cookie.name, cookie.value))
                .collect::<Vec<_>>()
                .join("; ")
        });
        (header, decision)
    }

    pub fn store_response(
        &self,
        request_url: &Url,
        context: &CookieRequestContext,
        values: impl IntoIterator<Item = String>,
    ) -> Result<(), CookieStoreError> {
        self.store(request_url, context, values, true)
    }

    fn store(
        &self,
        request_url: &Url,
        context: &CookieRequestContext,
        values: impl IntoIterator<Item = String>,
        http_api: bool,
    ) -> Result<(), CookieStoreError> {
        if matches!(context.credentials, CookieCredentials::Omit)
            || matches!(context.credentials, CookieCredentials::SameOrigin)
                && context
                    .initiator_url
                    .as_deref()
                    .and_then(|url| Url::parse(url).ok())
                    .is_some_and(|source| source.origin() != request_url.origin())
        {
            return Ok(());
        }
        let now = (self.now)();
        let host = request_url
            .host_str()
            .ok_or_else(|| CookieStoreError::Invalid("cookie URL has no host".into()))?
            .trim_end_matches('.')
            .to_ascii_lowercase();
        let unpartitioned = self.has_storage_access(request_url, context);
        let cross_site = !request_is_same_site(request_url, context);
        let mut cookies = self
            .cookies
            .lock()
            .map_err(|_| CookieStoreError::Persistent("cookie lock poisoned".into()))?;
        let mut next = cookies.clone();
        for value in values {
            // Invalid individual headers are ignored, not a reason to abort
            // response processing or lose later valid Set-Cookie headers.
            let Ok(parsed) = Cookie::parse(value.clone()) else {
                continue;
            };
            if parsed.name().len().saturating_add(parsed.value().len()) > 4096 {
                continue;
            }
            if !http_api && parsed.http_only().unwrap_or(false) {
                continue;
            }
            if parsed.domain().is_some_and(|domain| domain.ends_with('.')) {
                continue;
            }
            let host_only = parsed.domain().is_none();
            let domain = parsed
                .domain()
                .unwrap_or(&host)
                .trim_start_matches('.')
                .trim_end_matches('.')
                .to_ascii_lowercase();
            if !domain_matches_setting_host(&host, &domain)
                || (!host_only && is_public_suffix(&domain))
            {
                continue;
            }
            let secure = parsed.secure().unwrap_or(false);
            let partitioned = value.split(';').skip(1).any(|attribute| {
                attribute
                    .split('=')
                    .next()
                    .is_some_and(|name| name.trim().eq_ignore_ascii_case("partitioned"))
            });
            if !partitioned && !unpartitioned {
                continue;
            }
            let partition = if partitioned {
                if !secure {
                    continue;
                }
                let Some(key) = partition_key(request_url, context) else {
                    continue;
                };
                Some(key)
            } else {
                None
            };
            if secure && !is_potentially_trustworthy(request_url) {
                continue;
            }
            let same_site = match parsed.same_site() {
                Some(SameSite::Strict) => StoredSameSite::Strict,
                Some(SameSite::Lax) => StoredSameSite::Lax,
                Some(SameSite::None) if secure => StoredSameSite::None,
                Some(SameSite::None) => continue,
                None => StoredSameSite::Default,
            };
            if cross_site
                && !context.top_level_navigation
                && matches!(
                    same_site,
                    StoredSameSite::Strict | StoredSameSite::Lax | StoredSameSite::Default
                )
            {
                continue;
            }
            let path = parsed
                .path()
                .filter(|path| path.starts_with('/'))
                .map(str::to_owned)
                .unwrap_or_else(|| default_path(request_url.path()));
            let name = parsed.name().to_string();
            let prefix = name.to_ascii_lowercase();
            if (prefix.starts_with("__http-") || prefix.starts_with("__host-http-"))
                && (!http_api || !secure || !parsed.http_only().unwrap_or(false))
                || prefix.starts_with("__secure-") && !secure
                || prefix.starts_with("__host-")
                    && (!secure || !host_only || parsed.path() != Some("/"))
            {
                continue;
            }
            let expires_at_ms = if let Some(max_age) = parsed.max_age() {
                let seconds = max_age.whole_seconds();
                if seconds <= 0 {
                    Some(0)
                } else {
                    Some(now.saturating_add((seconds as u64).saturating_mul(1_000)))
                }
            } else {
                parsed.expires_datetime().map(|time| {
                    let seconds = time.unix_timestamp();
                    if seconds <= 0 {
                        0
                    } else {
                        (seconds as u64).saturating_mul(1_000)
                    }
                })
            };
            let mut stored = StoredCookie {
                partition_key: partition,
                last_access_ms: now,
                name,
                value: parsed.value().to_string(),
                domain,
                host_only,
                path,
                secure,
                http_only: parsed.http_only().unwrap_or(false),
                same_site,
                created_at_ms: Some(now),
                expires_at_ms: expires_at_ms
                    .map(|expiry| expiry.min(now.saturating_add(400 * 24 * 60 * 60 * 1000))),
            };
            if let Some(old) = next.get(&stored.key()).filter(|old| !old.expired(now)) {
                if !http_api && old.http_only {
                    continue;
                }
                stored.created_at_ms = old.created_at_ms;
            }
            // An insecure response must not overlay a more specific Secure
            // cookie, including deletion. The path comparison is asymmetric.
            if !secure
                && !is_potentially_trustworthy(request_url)
                && next.values().any(|old| {
                    old.secure
                        && !old.expired(now)
                        && old.name == stored.name
                        && old.partition_key == stored.partition_key
                        && (domain_matches_setting_host(&old.domain, &stored.domain)
                            || domain_matches_setting_host(&stored.domain, &old.domain))
                        && old.path_matches(&stored.path)
                })
            {
                continue;
            }
            if stored.expired(now) {
                next.remove(&stored.key());
            } else {
                next.insert(stored.key(), stored);
            }
        }
        enforce_cookie_limits(&mut next, now);
        self.persist_values(&next)?;
        *cookies = next;
        Ok(())
    }

    /// Authenticated automation sees associated HttpOnly cookies as well as
    /// script-visible cookies. Deletion has the same URL association boundary.
    pub fn webdriver_cookies(
        &self,
        url: &Url,
        delete: bool,
    ) -> Result<Vec<serde_json::Value>, CookieStoreError> {
        self.webdriver_cookies_with_context(
            url,
            &CookieRequestContext::navigation(Some(url.to_string())),
            delete,
        )
    }
    pub(crate) fn webdriver_cookies_with_context(
        &self,
        url: &Url,
        context: &CookieRequestContext,
        delete: bool,
    ) -> Result<Vec<serde_json::Value>, CookieStoreError> {
        let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
            return Ok(Vec::new());
        };
        if !matches!(url.scheme(), "http" | "https") {
            return Ok(Vec::new());
        }
        let now = (self.now)();
        let mut cookies = self
            .cookies
            .lock()
            .map_err(|_| CookieStoreError::Persistent("cookie store lock poisoned".into()))?;
        let selected = cookies
            .values()
            .filter(|cookie| {
                (cookie.partition_key.is_none()
                    || cookie.partition_key == partition_key(url, context))
                    && !cookie.expired(now)
                    && cookie.domain_matches(&host)
                    && cookie.path_matches(url.path())
                    && (!cookie.secure || is_potentially_trustworthy(url))
            })
            .cloned()
            .collect::<Vec<_>>();
        let result=selected.iter().map(|cookie|{
            let mut value=serde_json::json!({"name":cookie.name,"value":cookie.value,"path":cookie.path,"domain":cookie.domain,"secure":cookie.secure,"httpOnly":cookie.http_only,"sameSite":match cookie.same_site{StoredSameSite::Default|StoredSameSite::Lax=>"Lax",StoredSameSite::Strict=>"Strict",StoredSameSite::None=>"None"}});
            if let Some(expiry)=cookie.expires_at_ms{value["expiry"]=serde_json::json!(expiry/1000);}
            value
        }).collect();
        if delete {
            let mut next = cookies.clone();
            for cookie in selected {
                next.remove(&cookie.key());
            }
            self.persist_values(&next)?;
            *cookies = next;
        }
        Ok(result)
    }

    pub fn document_header(&self, url: &Url) -> Option<String> {
        self.document_header_with_context(
            url,
            &CookieRequestContext::navigation(Some(url.to_string())),
        )
    }
    pub(crate) fn document_header_with_context(
        &self,
        url: &Url,
        context: &CookieRequestContext,
    ) -> Option<String> {
        let unpartitioned = self.has_storage_access(url, context);
        let host = url.host_str()?.to_ascii_lowercase();
        let now = (self.now)();
        let mut cookies = self.cookies.lock().ok()?;
        let mut selected = cookies
            .values_mut()
            .filter(|cookie| {
                !cookie.http_only
                    && (cookie.partition_key.is_some() || unpartitioned)
                    && (cookie.partition_key.is_none()
                        || cookie.partition_key == partition_key(url, context))
                    && !cookie.expired(now)
                    && cookie.domain_matches(&host)
                    && cookie.path_matches(url.path())
                    && (!cookie.secure || is_potentially_trustworthy(url))
            })
            .map(|cookie| {
                cookie.last_access_ms = now;
                cookie.clone()
            })
            .collect::<Vec<_>>();
        selected.sort_by(|left, right| {
            right
                .path
                .len()
                .cmp(&left.path.len())
                .then(left.created_at_ms.cmp(&right.created_at_ms))
        });
        (!selected.is_empty()).then(|| {
            selected
                .into_iter()
                .map(|cookie| format!("{}={}", cookie.name, cookie.value))
                .collect::<Vec<_>>()
                .join("; ")
        })
    }

    pub fn store_document_cookie(
        &self,
        document_url: &Url,
        value: String,
    ) -> Result<(), CookieStoreError> {
        self.store_document_cookie_with_context(
            document_url,
            &CookieRequestContext {
                dns_timing: None,
                kind: CookieRequestKind::Other,
                initiator_url: Some(document_url.to_string()),
                top_level_url: Some(document_url.to_string()),
                ..Default::default()
            },
            value,
        )
    }
    pub(crate) fn store_document_cookie_with_context(
        &self,
        document_url: &Url,
        context: &CookieRequestContext,
        value: String,
    ) -> Result<(), CookieStoreError> {
        if value
            .split(';')
            .skip(1)
            .any(|attribute| attribute.trim().eq_ignore_ascii_case("httponly"))
        {
            return Ok(());
        }
        self.store(document_url, context, [value], false)
    }

    fn load(&self) -> Result<(), CookieStoreError> {
        let Some(database) = &self.database else {
            return Ok(());
        };
        let read = database
            .begin_read()
            .map_err(|error| CookieStoreError::Persistent(error.to_string()))?;
        if let Ok(table) = read.open_table(PERSISTENT_COOKIES_V2)
            && let Some(value) = table
                .get("state")
                .map_err(|error| CookieStoreError::Persistent(error.to_string()))?
        {
            let stored: Vec<StoredCookie> = serde_json::from_slice(value.value())
                .map_err(|error| CookieStoreError::Persistent(error.to_string()))?;
            let mut restored = stored
                .into_iter()
                .map(|cookie| (cookie.key(), cookie))
                .collect();
            enforce_cookie_limits(&mut restored, (self.now)());
            *self
                .cookies
                .lock()
                .map_err(|_| CookieStoreError::Persistent("cookie lock poisoned".into()))? =
                restored;
            return Ok(());
        }
        // Legacy records deliberately receive no creation time, so migration
        // cannot accidentally grant the Lax-allowing-unsafe exception.
        #[derive(Deserialize)]
        struct LegacyCookie {
            url: String,
            set_cookie: String,
        }
        let legacy = read
            .open_table(PERSISTENT_COOKIES_V1)
            .ok()
            .and_then(|table| table.get("state").ok().flatten())
            .and_then(|value| serde_json::from_slice::<Vec<LegacyCookie>>(value.value()).ok())
            .unwrap_or_default();
        drop(read);
        // Build the complete migration off to the side. Writing a V2 table
        // after each record would make an interrupted import look complete.
        let staged = Self {
            access: Mutex::new(StorageAccessPolicy::default()),
            cookies: Arc::new(Mutex::new(BTreeMap::new())),
            database: None,
            now: self.now.clone(),
            #[cfg(test)]
            fail_next_commit: std::sync::atomic::AtomicBool::new(false),
        };
        for record in legacy {
            if let Ok(url) = Url::parse(&record.url) {
                staged.store_response(
                    &url,
                    &CookieRequestContext::navigation(Some(url.to_string())),
                    [record.set_cookie],
                )?;
            }
        }
        let mut next = staged
            .cookies
            .lock()
            .map_err(|_| CookieStoreError::Persistent("cookie migration lock poisoned".into()))?
            .clone();
        for cookie in next.values_mut() {
            cookie.created_at_ms = None;
        }
        self.persist_values(&next)?;
        *self
            .cookies
            .lock()
            .map_err(|_| CookieStoreError::Persistent("cookie lock poisoned".into()))? = next;
        Ok(())
    }

    fn persist(&self) -> Result<(), CookieStoreError> {
        let cookies = self
            .cookies
            .lock()
            .map_err(|_| CookieStoreError::Persistent("cookie lock poisoned".into()))?;
        self.persist_values(&cookies)
    }
    fn persist_values(
        &self,
        cookies: &BTreeMap<(Option<String>, String, String, String), StoredCookie>,
    ) -> Result<(), CookieStoreError> {
        #[cfg(test)]
        if self
            .fail_next_commit
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(CookieStoreError::Persistent(
                "injected cookie commit failure".into(),
            ));
        }
        let Some(database) = &self.database else {
            return Ok(());
        };
        let now = (self.now)();
        let values = cookies
            .values()
            .filter(|cookie| cookie.expires_at_ms.is_some() && !cookie.expired(now))
            .cloned()
            .collect::<Vec<_>>();
        let encoded = serde_json::to_vec(&values)
            .map_err(|error| CookieStoreError::Persistent(error.to_string()))?;
        let write = database
            .begin_write()
            .map_err(|error| CookieStoreError::Persistent(error.to_string()))?;
        {
            let mut table = write
                .open_table(PERSISTENT_COOKIES_V2)
                .map_err(|error| CookieStoreError::Persistent(error.to_string()))?;
            table
                .insert("state", encoded.as_slice())
                .map_err(|error| CookieStoreError::Persistent(error.to_string()))?;
        }
        write
            .commit()
            .map_err(|error| CookieStoreError::Persistent(error.to_string()))
    }
}

const COOKIES_PER_PARTITION_DOMAIN: usize = 180;
const COOKIES_TOTAL: usize = 3000;
fn enforce_cookie_limits(
    cookies: &mut BTreeMap<(Option<String>, String, String, String), StoredCookie>,
    now: u64,
) {
    cookies.retain(|_, cookie| !cookie.expired(now));
    let mut buckets: BTreeMap<(Option<String>, String), Vec<_>> = BTreeMap::new();
    for (key, cookie) in cookies.iter() {
        buckets
            .entry((cookie.partition_key.clone(), cookie.domain.clone()))
            .or_default()
            .push((
                cookie.secure,
                cookie.last_access_ms,
                cookie.created_at_ms,
                key.clone(),
            ));
    }
    for entries in buckets.values_mut() {
        entries.sort();
        let excess = entries.len().saturating_sub(COOKIES_PER_PARTITION_DOMAIN);
        for (_, _, _, key) in entries.iter().take(excess) {
            cookies.remove(key);
        }
    }
    if cookies.len() > COOKIES_TOTAL {
        let mut entries = cookies
            .iter()
            .map(|(key, cookie)| (cookie.last_access_ms, cookie.created_at_ms, key.clone()))
            .collect::<Vec<_>>();
        entries.sort();
        for (_, _, key) in entries.into_iter().take(cookies.len() - COOKIES_TOTAL) {
            cookies.remove(&key);
        }
    }
}

fn partition_key(request_url: &Url, context: &CookieRequestContext) -> Option<String> {
    let top = if context.top_level_navigation {
        request_url.clone()
    } else {
        context
            .top_level_url
            .as_deref()
            .map(Url::parse)
            .transpose()
            .ok()?
            .unwrap_or_else(|| request_url.clone())
    };
    let site = SchemefulSite::from_url(&top)?;
    let cross_ancestor = !context.top_level_navigation
        && context.ancestor_urls.iter().any(|url| {
            Url::parse(url)
                .ok()
                .and_then(|url| SchemefulSite::from_url(&url))
                .as_ref()
                != Some(&site)
        });
    serde_json::to_string(&(site, cross_ancestor)).ok()
}

fn request_is_same_site(request_url: &Url, context: &CookieRequestContext) -> bool {
    if context.redirect_chain_cross_site {
        return false;
    }
    let Some(target) = SchemefulSite::from_url(request_url) else {
        return false;
    };
    let source = context
        .top_level_url
        .as_deref()
        .or(context.initiator_url.as_deref())
        .and_then(|url| Url::parse(url).ok())
        .and_then(|url| SchemefulSite::from_url(&url));
    let Some(source) = source else {
        return context.top_level_url.is_none() && context.initiator_url.is_none();
    };
    source == target
        && context.ancestor_urls.iter().all(|url| {
            Url::parse(url)
                .ok()
                .and_then(|url| SchemefulSite::from_url(&url))
                .is_some_and(|site| site == target)
        })
}

fn same_site_allows_cross_site(
    cookie: &StoredCookie,
    context: &CookieRequestContext,
    safe_method: bool,
    now_ms: u64,
) -> bool {
    match cookie.same_site {
        StoredSameSite::None => true,
        StoredSameSite::Strict => false,
        StoredSameSite::Lax => context.top_level_navigation && safe_method,
        StoredSameSite::Default => {
            context.top_level_navigation
                && (safe_method
                    || cookie.created_at_ms.is_some_and(|created| {
                        now_ms.saturating_sub(created) <= LAX_ALLOW_UNSAFE_MS
                    }))
        }
    }
}

fn is_safe_method(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "OPTIONS" | "TRACE"
    )
}

fn is_potentially_trustworthy(url: &Url) -> bool {
    matches!(url.scheme(), "https" | "wss")
        || url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        })
}

fn domain_matches_setting_host(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn is_public_suffix(domain: &str) -> bool {
    psl::suffix_str(domain).is_some_and(|suffix| suffix == domain)
}

fn default_path(path: &str) -> String {
    if !path.starts_with('/') || path == "/" {
        return "/".into();
    }
    path.rsplit_once('/')
        .map_or(
            "/",
            |(prefix, _)| {
                if prefix.is_empty() { "/" } else { prefix }
            },
        )
        .into()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn context(top: &str, navigation: bool) -> CookieRequestContext {
        CookieRequestContext {
            dns_timing: None,
            top_level_url: Some(top.into()),
            initiator_url: Some(top.into()),
            top_level_navigation: navigation,
            kind: if navigation {
                CookieRequestKind::Document
            } else {
                CookieRequestKind::Fetch
            },
            ..CookieRequestContext::default()
        }
    }

    #[test]
    fn schemeful_sites_use_the_registrable_domain() {
        let a = SchemefulSite::from_url(&Url::parse("https://a.shop.example.co.uk/x").unwrap());
        let b = SchemefulSite::from_url(&Url::parse("https://b.example.co.uk/y").unwrap());
        let insecure = SchemefulSite::from_url(&Url::parse("http://b.example.co.uk/y").unwrap());
        assert_eq!(a, b);
        assert_ne!(a, insecure);
    }

    #[test]
    fn strict_lax_none_and_default_are_context_sensitive() {
        let clock = Arc::new(AtomicU64::new(1_000));
        let store = ContextCookieStore::memory_with_clock({
            let clock = clock.clone();
            Arc::new(move || clock.load(Ordering::Relaxed))
        });
        // Isolate SameSite behavior from the independently configurable privacy profile.
        store.set_third_party_policy(ThirdPartyCookiePolicy::Allow);
        let origin = Url::parse("https://auth.example.test/login").unwrap();
        store
            .store_response(
                &origin,
                &context(origin.as_str(), true),
                [
                    "strict=1; SameSite=Strict; Secure; Path=/".into(),
                    "lax=1; SameSite=Lax; Secure; Path=/".into(),
                    "none=1; SameSite=None; Secure; Path=/".into(),
                    "default=1; Secure; Path=/".into(),
                ],
            )
            .unwrap();
        let cross = context("https://rp.other.test/", false);
        let (header, _) = store.header_for_method(&origin, "GET", &cross);
        assert_eq!(header.as_deref(), Some("none=1"));
        let navigation = context("https://rp.other.test/", true);
        let (header, _) = store.header_for_method(&origin, "POST", &navigation);
        let header = header.unwrap();
        assert!(header.contains("none=1"));
        assert!(header.contains("default=1"));
        assert!(!header.contains("lax=1"));
        clock.store(121_001, Ordering::Relaxed);
        let (header, _) = store.header_for_method(&origin, "POST", &navigation);
        assert_eq!(header.as_deref(), Some("none=1"));
        let (header, _) = store.header_for_method(&origin, "GET", &navigation);
        let header = header.unwrap();
        assert!(header.contains("lax=1"));
        assert!(header.contains("default=1"));
    }

    #[test]
    fn none_requires_secure_and_prefixes_are_enforced() {
        let store = ContextCookieStore::memory();
        let url = Url::parse("https://example.test/").unwrap();
        store
            .store_response(
                &url,
                &context(url.as_str(), true),
                [
                    "bad=1; SameSite=None".into(),
                    "__Secure-bad=1".into(),
                    "__Host-bad=1; Secure; Domain=example.test; Path=/".into(),
                    "__Host-good=1; Secure; Path=/".into(),
                ],
            )
            .unwrap();
        let (header, _) = store.header(&url, &context(url.as_str(), true));
        assert_eq!(header.as_deref(), Some("__Host-good=1"));
    }

    #[test]
    fn redirect_chain_cannot_launder_same_site() {
        let store = ContextCookieStore::memory();
        let url = Url::parse("https://example.test/").unwrap();
        store
            .store_response(
                &url,
                &context(url.as_str(), true),
                ["strict=1; SameSite=Strict; Secure".into()],
            )
            .unwrap();
        let mut redirected = context(url.as_str(), true);
        redirected.redirect_chain_cross_site = true;
        let (header, _) = store.header(&url, &redirected);
        assert!(header.is_none());
    }

    #[test]
    fn document_cookie_cannot_read_or_create_http_only_values() {
        let store = ContextCookieStore::memory();
        let url = Url::parse("https://app.example/").unwrap();
        let context = CookieRequestContext::navigation(Some(url.to_string()));
        store
            .store_response(&url, &context, ["secret=one; Secure; HttpOnly".into()])
            .unwrap();
        assert_eq!(store.document_header(&url), None);
        assert!(
            store
                .header(&url, &context)
                .0
                .unwrap()
                .contains("secret=one")
        );
        store
            .store_document_cookie(&url, "visible=two; Secure".into())
            .unwrap();
        assert_eq!(store.document_header(&url).as_deref(), Some("visible=two"));
        store
            .store_document_cookie(&url, "forged=three; HttpOnly".into())
            .unwrap();
        assert!(!store.header(&url, &context).0.unwrap().contains("forged="));
    }
    #[test]
    fn webdriver_cookie_scope_includes_http_only_and_deletes_only_associated_cookies() {
        let store = ContextCookieStore::memory();
        let url = Url::parse("https://app.example.test/private/page").unwrap();
        store
            .store_response(
                &url,
                &context(url.as_str(), true),
                [
                    "secret=one; Secure; HttpOnly; Path=/private; SameSite=Strict".into(),
                    "public=two; Secure; Path=/".into(),
                    "other=three; Secure; Path=/other".into(),
                ],
            )
            .unwrap();
        let values = store.webdriver_cookies(&url, false).unwrap();
        assert_eq!(values.len(), 2);
        let secret = values
            .iter()
            .find(|value| value["name"] == "secret")
            .unwrap();
        assert_eq!(secret["httpOnly"], true);
        assert_eq!(secret["sameSite"], "Strict");
        assert!(!store.document_header(&url).unwrap().contains("secret"));
        assert_eq!(store.webdriver_cookies(&url, true).unwrap().len(), 2);
        assert!(store.webdriver_cookies(&url, false).unwrap().is_empty());
        let other = store
            .webdriver_cookies(
                &Url::parse("https://app.example.test/other/page").unwrap(),
                false,
            )
            .unwrap();
        assert_eq!(other.len(), 1);
        assert_eq!(other[0]["name"], "other");
        assert!(
            store
                .webdriver_cookies(
                    &Url::parse("https://foreign.example.test/other/page").unwrap(),
                    false
                )
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn webdriver_cookie_expiry_and_secure_transport_are_not_bypassed() {
        let clock = Arc::new(AtomicU64::new(1000));
        let store = ContextCookieStore::memory_with_clock({
            let clock = clock.clone();
            Arc::new(move || clock.load(Ordering::Relaxed))
        });
        let url = Url::parse("https://app.example.test/").unwrap();
        store
            .store_response(
                &url,
                &context(url.as_str(), true),
                ["short=one; Secure; Max-Age=1".into()],
            )
            .unwrap();
        assert_eq!(
            store.webdriver_cookies(&url, false).unwrap()[0]["expiry"],
            2
        );
        assert!(
            store
                .webdriver_cookies(&Url::parse("http://app.example.test/").unwrap(), false)
                .unwrap()
                .is_empty()
        );
        clock.store(2001, Ordering::Relaxed);
        assert!(store.webdriver_cookies(&url, false).unwrap().is_empty());
    }
}

#[cfg(test)]
mod form_source_site_tests {
    use super::*;
    #[test]
    fn opaque_or_cross_site_form_source_does_not_become_destination_same_site() {
        let target = Url::parse("https://top.test/submit").unwrap();
        let mut context = CookieRequestContext::navigation(Some("https://top.test/".into()));
        context.initiator_url = Some("https://child.test/form".into());
        context.ancestor_urls = vec!["https://child.test/form".into(), "https://top.test/".into()];
        assert!(!request_is_same_site(&target, &context));
        context.ancestor_urls = vec!["https://top.test/form".into()];
        assert!(request_is_same_site(&target, &context));
        context.top_level_url = Some("about:blank".into());
        context.ancestor_urls.clear();
        assert!(!request_is_same_site(&target, &context));
    }
}

#[cfg(test)]
mod cookie_admission_fault_regressions {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[test]
    fn document_setter_cannot_replace_or_delete_http_only_cookie() {
        let store = ContextCookieStore::memory();
        let url = Url::parse("https://a.test/app").unwrap();
        let context = CookieRequestContext::navigation(Some(url.to_string()));
        store
            .store_response(&url, &context, ["sid=server; HttpOnly; Path=/".into()])
            .unwrap();
        for value in ["sid=script; Path=/", "sid=; Max-Age=0; Path=/"] {
            store.store_document_cookie(&url, value.into()).unwrap();
        }
        assert_eq!(
            store.header(&url, &context).0.as_deref(),
            Some("sid=server")
        );
        assert!(store.document_header(&url).is_none());
        store
            .store_response(&url, &context, ["sid=rotated; HttpOnly; Path=/".into()])
            .unwrap();
        assert_eq!(
            store.header(&url, &context).0.as_deref(),
            Some("sid=rotated")
        );
    }
    #[test]
    fn insecure_overlay_and_implicit_host_prefix_path_are_rejected() {
        let store = ContextCookieStore::memory();
        let secure = Url::parse("https://a.test/login").unwrap();
        let insecure = Url::parse("http://a.test/login").unwrap();
        store
            .store_response(
                &secure,
                &CookieRequestContext::navigation(Some(secure.to_string())),
                [
                    "sid=secure; Secure; Path=/login".into(),
                    "__Host-missing=value; Secure".into(),
                    "__Host-valid=value; Secure; Path=/".into(),
                ],
            )
            .unwrap();
        store
            .store_response(
                &insecure,
                &CookieRequestContext::navigation(Some(insecure.to_string())),
                [
                    "sid=overlay; Path=/login/child".into(),
                    "sid=; Max-Age=0; Path=/login".into(),
                ],
            )
            .unwrap();
        let state = store.cookies.lock().unwrap();
        assert_eq!(
            state.values().filter(|cookie| cookie.name == "sid").count(),
            1
        );
        assert!(!state.values().any(|cookie| cookie.name == "__Host-missing"));
        assert!(state.values().any(|cookie| cookie.name == "__Host-valid"));
    }
    #[test]
    fn replacement_preserves_age_and_failed_commit_preserves_memory() {
        let clock = Arc::new(AtomicU64::new(1000));
        let source = clock.clone();
        let store =
            ContextCookieStore::memory_with_clock(Arc::new(move || source.load(Ordering::SeqCst)));
        let url = Url::parse("https://a.test/").unwrap();
        let context = CookieRequestContext::navigation(Some(url.to_string()));
        store
            .store_response(&url, &context, ["a=old; Max-Age=3600".into()])
            .unwrap();
        clock.store(200000, Ordering::SeqCst);
        store
            .store_response(&url, &context, ["a=new; Max-Age=3600".into()])
            .unwrap();
        assert_eq!(
            store
                .cookies
                .lock()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .created_at_ms,
            Some(1000)
        );
        store.fail_next_commit.store(true, Ordering::SeqCst);
        assert!(
            store
                .store_response(
                    &url,
                    &context,
                    ["a=lost; Max-Age=3600".into(), "b=lost; Max-Age=3600".into()]
                )
                .is_err()
        );
        assert_eq!(store.header(&url, &context).0.as_deref(), Some("a=new"));
        store.fail_next_commit.store(true, Ordering::SeqCst);
        assert!(store.webdriver_cookies(&url, true).is_err());
        assert_eq!(store.header(&url, &context).0.as_deref(), Some("a=new"));
        store
            .store_response(
                &url,
                &context,
                [
                    "invalid".into(),
                    format!("large={}", "x".repeat(4096)),
                    "b=valid".into(),
                ],
            )
            .unwrap();
        assert!(
            store
                .cookies
                .lock()
                .unwrap()
                .values()
                .any(|cookie| cookie.name == "b")
        );
        assert!(
            !store
                .cookies
                .lock()
                .unwrap()
                .values()
                .any(|cookie| cookie.name == "large")
        );
    }
}

#[cfg(test)]
mod partitioned_cookie_regressions {
    use super::*;
    fn embedded(top: &str, ancestors: &[&str]) -> CookieRequestContext {
        CookieRequestContext {
            dns_timing: None,
            top_level_url: Some(top.into()),
            ancestor_urls: ancestors.iter().map(|value| value.to_string()).collect(),
            ..Default::default()
        }
    }
    #[test]
    fn partitioned_cookie_keys_isolate_top_sites_and_exact_deletion() {
        let store = ContextCookieStore::memory();
        let url = Url::parse("https://third.example/resource").unwrap();
        let a = embedded("https://a.example/", &["https://third.example/"]);
        let b = embedded("https://b.example/", &["https://third.example/"]);
        for (context, value) in [(&a, "a"), (&b, "b")] {
            store
                .store_document_cookie_with_context(
                    &url,
                    context,
                    format!("id={value}; Secure; SameSite=None; Partitioned; Path=/"),
                )
                .unwrap();
        }
        assert_eq!(
            store.document_header_with_context(&url, &a).as_deref(),
            Some("id=a")
        );
        assert_eq!(
            store.document_header_with_context(&url, &b).as_deref(),
            Some("id=b")
        );
        assert_eq!(
            store
                .webdriver_cookies_with_context(&url, &a, true)
                .unwrap()[0]["value"],
            "a"
        );
        assert!(store.document_header_with_context(&url, &a).is_none());
        assert_eq!(
            store.document_header_with_context(&url, &b).as_deref(),
            Some("id=b")
        );
    }
    #[test]
    fn ancestor_bit_separates_same_site_from_cross_site_round_trip() {
        let store = ContextCookieStore::memory();
        let url = Url::parse("https://a.example/").unwrap();
        let direct = embedded(url.as_str(), &[url.as_str()]);
        let nested = embedded(url.as_str(), &["https://b.example/", url.as_str()]);
        store
            .store_document_cookie_with_context(
                &url,
                &direct,
                "id=direct; Secure; SameSite=None; Partitioned".into(),
            )
            .unwrap();
        assert!(store.document_header_with_context(&url, &nested).is_none());
        store
            .store_document_cookie_with_context(
                &url,
                &nested,
                "id=nested; Secure; SameSite=None; Partitioned".into(),
            )
            .unwrap();
        assert_eq!(store.header(&url, &direct).0.as_deref(), Some("id=direct"));
        assert_eq!(store.header(&url, &nested).0.as_deref(), Some("id=nested"));
    }
    #[test]
    fn embedded_partitioned_creation_requires_secure_and_same_site_none() {
        let store = ContextCookieStore::memory();
        let url = Url::parse("https://third.example/").unwrap();
        let context = embedded("https://top.example/", &[url.as_str()]);
        for value in [
            "unset=0; Secure; Partitioned",
            "strict=0; Secure; Partitioned; SameSite=Strict",
            "lax=0; Secure; Partitioned; SameSite=Lax",
            "insecure=0; Partitioned; SameSite=None",
            "none=0; Secure; Partitioned; SameSite=None",
        ] {
            store
                .store_document_cookie_with_context(&url, &context, value.into())
                .unwrap();
        }
        let cookies = store
            .webdriver_cookies_with_context(&url, &context, false)
            .unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0]["name"], "none");
    }
    #[test]
    fn stored_partition_round_trip_and_legacy_unpartitioned_decode() {
        let store = ContextCookieStore::memory();
        let url = Url::parse("https://a.example/").unwrap();
        let context = CookieRequestContext::navigation(Some(url.to_string()));
        store
            .store_response(
                &url,
                &context,
                ["id=persistent; Secure; Partitioned; Max-Age=3600".into()],
            )
            .unwrap();
        let state = store.cookies.lock().unwrap();
        let cookie = state.values().next().unwrap();
        let mut value = serde_json::to_value(cookie).unwrap();
        let restored: StoredCookie = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(restored.key(), cookie.key());
        assert!(restored.partition_key.is_some());
        value.as_object_mut().unwrap().remove("partition_key");
        let legacy: StoredCookie = serde_json::from_value(value).unwrap();
        assert!(legacy.partition_key.is_none());
    }
}

#[cfg(test)]
mod cookie_migration_fault_regressions {
    use super::*;
    #[test]
    fn failed_legacy_migration_does_not_publish_partial_v2_state() {
        let path = std::env::temp_dir().join(format!(
            "oryn-cookie-migration-{}-{}.redb",
            std::process::id(),
            now_ms()
        ));
        {
            let database = Arc::new(Database::create(&path).unwrap());
            let encoded=serde_json::to_vec(&serde_json::json!([{"url":"https://a.example/","set_cookie":"a=one; Max-Age=3600"},{"url":"https://a.example/","set_cookie":"b=two; Max-Age=3600"}])).unwrap();
            let write = database.begin_write().unwrap();
            {
                let mut table = write.open_table(PERSISTENT_COOKIES_V1).unwrap();
                table.insert("state", encoded.as_slice()).unwrap();
            }
            write.commit().unwrap();
            let store = ContextCookieStore {
                access: Mutex::new(StorageAccessPolicy::default()),
                cookies: Arc::new(Mutex::new(BTreeMap::new())),
                database: Some(database.clone()),
                now: Arc::new(now_ms),
                fail_next_commit: std::sync::atomic::AtomicBool::new(true),
            };
            assert!(store.load().is_err());
            assert!(store.cookies.lock().unwrap().is_empty());
            {
                let read = database.begin_read().unwrap();
                assert!(read.open_table(PERSISTENT_COOKIES_V2).is_err());
            }
            store.load().unwrap();
            let values = store.cookies.lock().unwrap();
            assert_eq!(values.len(), 2);
            assert!(values.values().all(|cookie| cookie.created_at_ms.is_none()));
        }
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod cookie_capacity_regressions {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[test]
    fn domain_capacity_keeps_recently_read_cookie_and_preserves_other_partition() {
        let clock = Arc::new(AtomicU64::new(1));
        let source = clock.clone();
        let store =
            ContextCookieStore::memory_with_clock(Arc::new(move || source.load(Ordering::SeqCst)));
        let url = Url::parse("https://a.example/").unwrap();
        let context = CookieRequestContext::navigation(Some(url.to_string()));
        store
            .store_response(
                &url,
                &context,
                (0..COOKIES_PER_PARTITION_DOMAIN)
                    .map(|index| format!("c{index:03}=value; Path=/p{index:03}")),
            )
            .unwrap();
        clock.store(2, Ordering::SeqCst);
        assert_eq!(
            store
                .document_header(&url.join("/p000").unwrap())
                .as_deref(),
            Some("c000=value")
        );
        clock.store(3, Ordering::SeqCst);
        store
            .store_response(
                &url,
                &context,
                [
                    "new=value; Path=/new".into(),
                    "separate=value; Secure; Partitioned; Path=/".into(),
                ],
            )
            .unwrap();
        let cookies = store.cookies.lock().unwrap();
        assert_eq!(cookies.len(), COOKIES_PER_PARTITION_DOMAIN + 1);
        assert!(cookies.values().any(|cookie| cookie.name == "c000"));
        assert!(!cookies.values().any(|cookie| cookie.name == "c001"));
        assert!(cookies.values().any(|cookie| cookie.name == "separate"));
    }
    #[test]
    fn total_capacity_and_expiration_limit_bound_persistent_state() {
        let store = ContextCookieStore::memory_with_clock(Arc::new(|| 1000));
        let url = Url::parse("https://a.example/").unwrap();
        let context = CookieRequestContext::navigation(Some(url.to_string()));
        store
            .store_response(&url, &context, ["sample=v; Max-Age=9999999999".into()])
            .unwrap();
        let sample = store
            .cookies
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .clone();
        assert_eq!(sample.expires_at_ms, Some(1000 + 400 * 24 * 60 * 60 * 1000));
        let mut entries = BTreeMap::new();
        for index in 0..=COOKIES_TOTAL {
            let mut cookie = sample.clone();
            cookie.domain = format!("d{index}.example");
            cookie.last_access_ms = index as u64;
            entries.insert(cookie.key(), cookie);
        }
        enforce_cookie_limits(&mut entries, 1000);
        assert_eq!(entries.len(), COOKIES_TOTAL);
        assert!(!entries.values().any(|cookie| cookie.domain == "d0.example"));
        enforce_cookie_limits(&mut entries, u64::MAX);
        assert!(entries.is_empty());
    }
}

#[cfg(test)]
mod http_cookie_prefix_regressions {
    use super::*;
    #[test]
    fn http_prefixes_require_native_http_setter_secure_and_http_only() {
        let store = ContextCookieStore::memory();
        let url = Url::parse("https://a.example/").unwrap();
        let context = CookieRequestContext::navigation(Some(url.to_string()));
        for prefix in ["__Http-", "__Host-Http-"] {
            store
                .store_document_cookie(&url, format!("{prefix}dom=v; Secure; Path=/"))
                .unwrap();
            store
                .store_response(
                    &url,
                    &context,
                    [
                        format!("{prefix}no-http=v; Secure; Path=/"),
                        format!("{prefix}no-secure=v; HttpOnly; Path=/"),
                        format!("{prefix}valid=v; HttpOnly; Secure; Path=/"),
                    ],
                )
                .unwrap();
        }
        let cookies = store.cookies.lock().unwrap();
        assert_eq!(cookies.len(), 2);
        assert!(
            cookies
                .values()
                .all(|cookie| cookie.name.ends_with("valid"))
        );
        drop(cookies);
        assert!(store.document_header(&url).is_none());
    }
}

#[cfg(test)]
mod third_party_policy_regressions {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    fn embedded(top: &Url, url: &Url) -> CookieRequestContext {
        CookieRequestContext {
            top_level_url: Some(top.to_string()),
            initiator_url: Some(url.to_string()),
            ancestor_urls: vec![url.to_string()],
            ..Default::default()
        }
    }
    #[test]
    fn default_block_and_grant_apply_to_http_and_dom_without_weakening_credentials_or_chips() {
        let store = ContextCookieStore::memory();
        let url = Url::parse("https://embed.example/").unwrap();
        let top = Url::parse("https://top.example/").unwrap();
        let first = CookieRequestContext::navigation(Some(url.to_string()));
        let context = embedded(&top, &url);
        store
            .store_response(
                &url,
                &first,
                [
                    "sid=first; Secure; SameSite=None; Path=/".into(),
                    "strict=first; Secure; SameSite=Strict; Path=/".into(),
                ],
            )
            .unwrap();
        assert!(!store.has_storage_access(&url, &context));
        assert!(store.header(&url, &context).0.is_none());
        assert!(store.document_header_with_context(&url, &context).is_none());
        store
            .store_response(
                &url,
                &context,
                ["sid=blocked; Secure; SameSite=None; Path=/".into()],
            )
            .unwrap();
        store
            .store_document_cookie_with_context(
                &url,
                &context,
                "dom=blocked; Secure; SameSite=None; Path=/".into(),
            )
            .unwrap();
        assert!(store.document_header(&url).unwrap().contains("sid=first"));
        assert!(!store.document_header(&url).unwrap().contains("dom="));
        store
            .store_response(
                &url,
                &context,
                ["partition=one; Secure; SameSite=None; Partitioned; Path=/".into()],
            )
            .unwrap();
        assert_eq!(
            store.header(&url, &context).0.as_deref(),
            Some("partition=one")
        );
        store.grant_storage_access(&top, &url).unwrap();
        assert!(store.has_storage_access(&url, &context));
        let header = store.header(&url, &context).0.unwrap();
        assert!(header.contains("sid=first"));
        assert!(!header.contains("strict="));
        let mut omitted = context.clone();
        omitted.credentials = CookieCredentials::Omit;
        assert!(store.header(&url, &omitted).0.is_none());
        let other = Url::parse("https://other.example/").unwrap();
        assert!(!store.has_storage_access(&url, &embedded(&other, &url)));
        let sibling = Url::parse("https://sub.embed.example/").unwrap();
        assert!(!store.has_storage_access(&sibling, &embedded(&top, &sibling)));
        store
            .store_document_cookie_with_context(
                &url,
                &context,
                "dom=allowed; Secure; SameSite=None; Path=/".into(),
            )
            .unwrap();
        assert!(
            store
                .document_header_with_context(&url, &context)
                .unwrap()
                .contains("dom=allowed")
        );
    }
    #[test]
    fn grants_expire_and_context_views_share_cookie_data_but_never_consent() {
        let clock = Arc::new(AtomicU64::new(1));
        let source = clock.clone();
        let profile =
            ContextCookieStore::memory_with_clock(Arc::new(move || source.load(Ordering::SeqCst)));
        let a = profile.for_context(ThirdPartyCookiePolicy::Block);
        let b = profile.for_context(ThirdPartyCookiePolicy::Block);
        let permissive = profile.for_context(ThirdPartyCookiePolicy::Allow);
        let url = Url::parse("https://embed.example/").unwrap();
        let top = Url::parse("https://top.example/").unwrap();
        let context = embedded(&top, &url);
        a.store_document_cookie(&url, "sid=shared; Secure; SameSite=None; Path=/".into())
            .unwrap();
        a.grant_storage_access(&top, &url).unwrap();
        assert!(a.has_storage_access(&url, &context));
        assert!(!b.has_storage_access(&url, &context));
        assert!(permissive.has_storage_access(&url, &context));
        assert!(b.document_header(&url).unwrap().contains("sid=shared"));
        clock.store(30 * 24 * 60 * 60 * 1000 + 2, Ordering::SeqCst);
        assert!(!a.has_storage_access(&url, &context));
        assert!(a.document_header(&url).is_some());
        assert!(
            a.grant_storage_access(&Url::parse("data:,opaque").unwrap(), &url)
                .is_err()
        );
        for i in 0..1024 {
            a.grant_storage_access(
                &Url::parse(&format!("https://site{i}.test/")).unwrap(),
                &url,
            )
            .unwrap();
        }
        assert!(a.grant_storage_access(&top, &url).is_err());
    }
}
