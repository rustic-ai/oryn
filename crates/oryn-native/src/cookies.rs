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
    Fetch,
    Xhr,
    WebSocket,
    #[default]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CookieRequestContext {
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
    name: String,
    value: String,
    domain: String,
    host_only: bool,
    path: String,
    secure: bool,
    http_only: bool,
    same_site: StoredSameSite,
    created_at_ms: Option<u64>,
    expires_at_ms: Option<u64>,
}

impl StoredCookie {
    fn key(&self) -> (String, String, String) {
        (self.domain.clone(), self.path.clone(), self.name.clone())
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

pub struct ContextCookieStore {
    cookies: Mutex<BTreeMap<(String, String, String), StoredCookie>>,
    database: Option<Arc<Database>>,
    now: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl ContextCookieStore {
    pub fn memory() -> Arc<Self> {
        Arc::new(Self {
            cookies: Mutex::new(BTreeMap::new()),
            database: None,
            now: Arc::new(now_ms),
        })
    }

    pub fn persistent(database: Arc<Database>) -> Result<Arc<Self>, CookieStoreError> {
        let store = Arc::new(Self {
            cookies: Mutex::new(BTreeMap::new()),
            database: Some(database),
            now: Arc::new(now_ms),
        });
        store.load()?;
        Ok(store)
    }

    #[cfg(test)]
    fn memory_with_clock(now: Arc<dyn Fn() -> u64 + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            cookies: Mutex::new(BTreeMap::new()),
            database: None,
            now,
        })
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
        let same_site = request_is_same_site(request_url, context);
        let safe_method = is_safe_method(method);
        let now = (self.now)();
        let mut selected = Vec::new();
        if let Ok(cookies) = self.cookies.lock() {
            for cookie in cookies.values() {
                let reason = if cookie.expired(now) {
                    Some("expired")
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
        if matches!(context.credentials, CookieCredentials::Omit) {
            return Ok(());
        }
        let now = (self.now)();
        let host = request_url
            .host_str()
            .ok_or_else(|| CookieStoreError::Invalid("cookie URL has no host".into()))?
            .trim_end_matches('.')
            .to_ascii_lowercase();
        let cross_site = !request_is_same_site(request_url, context);
        let mut cookies = self
            .cookies
            .lock()
            .map_err(|_| CookieStoreError::Persistent("cookie lock poisoned".into()))?;
        for value in values {
            let parsed = Cookie::parse(value.clone())
                .map_err(|error| CookieStoreError::Invalid(error.to_string()))?;
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
                && matches!(same_site, StoredSameSite::Strict | StoredSameSite::Lax)
            {
                continue;
            }
            let path = parsed
                .path()
                .filter(|path| path.starts_with('/'))
                .map(str::to_owned)
                .unwrap_or_else(|| default_path(request_url.path()));
            let name = parsed.name().to_string();
            if name.starts_with("__Secure-") && !secure
                || name.starts_with("__Host-") && (!secure || !host_only || path != "/")
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
                parsed.expires_datetime().and_then(|time| {
                    u64::try_from(time.unix_timestamp())
                        .ok()
                        .map(|value| value * 1_000)
                })
            };
            let stored = StoredCookie {
                name,
                value: parsed.value().to_string(),
                domain,
                host_only,
                path,
                secure,
                http_only: parsed.http_only().unwrap_or(false),
                same_site,
                created_at_ms: Some(now),
                expires_at_ms,
            };
            if stored.expired(now) {
                cookies.remove(&stored.key());
            } else {
                cookies.insert(stored.key(), stored);
            }
        }
        drop(cookies);
        self.persist()
    }

    pub fn document_header(&self, url: &Url) -> Option<String> {
        let host = url.host_str()?.to_ascii_lowercase();
        let now = (self.now)();
        let cookies = self.cookies.lock().ok()?;
        let values = cookies
            .values()
            .filter(|cookie| {
                !cookie.http_only
                    && !cookie.expired(now)
                    && cookie.domain_matches(&host)
                    && cookie.path_matches(url.path())
                    && (!cookie.secure || is_potentially_trustworthy(url))
            })
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>();
        (!values.is_empty()).then(|| values.join("; "))
    }

    pub fn store_document_cookie(
        &self,
        document_url: &Url,
        value: String,
    ) -> Result<(), CookieStoreError> {
        if value
            .split(';')
            .skip(1)
            .any(|attribute| attribute.trim().eq_ignore_ascii_case("httponly"))
        {
            return Ok(());
        }
        self.store_response(
            document_url,
            &CookieRequestContext {
                kind: CookieRequestKind::Other,
                initiator_url: Some(document_url.to_string()),
                top_level_url: Some(document_url.to_string()),
                ..CookieRequestContext::default()
            },
            [value],
        )
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
            self.cookies
                .lock()
                .map_err(|_| CookieStoreError::Persistent("cookie lock poisoned".into()))?
                .extend(stored.into_iter().map(|cookie| (cookie.key(), cookie)));
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
        for record in legacy {
            if let Ok(url) = Url::parse(&record.url) {
                let _ = self.store_response(
                    &url,
                    &CookieRequestContext::navigation(Some(url.to_string())),
                    [record.set_cookie],
                );
            }
        }
        if let Ok(mut cookies) = self.cookies.lock() {
            for cookie in cookies.values_mut() {
                cookie.created_at_ms = None;
            }
        }
        self.persist()
    }

    fn persist(&self) -> Result<(), CookieStoreError> {
        let Some(database) = &self.database else {
            return Ok(());
        };
        let now = (self.now)();
        let values = self
            .cookies
            .lock()
            .map_err(|_| CookieStoreError::Persistent("cookie lock poisoned".into()))?
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
    let Some(source) = source else { return true };
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
}
