//! Browser-owned Fetch Metadata. Author headers never choose these values.
use crate::cookies::{CookieRequestContext, CookieRequestKind, SchemefulSite};
use serde::{Deserialize, Serialize};
use url::Url;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Cors,
    NoCors,
    SameOrigin,
    Navigate,
    Websocket,
}
impl Mode {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "cors" => Self::Cors,
            "no-cors" => Self::NoCors,
            "same-origin" => Self::SameOrigin,
            "navigate" => Self::Navigate,
            "websocket" => Self::Websocket,
            _ => return None,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Self::Cors => "cors",
            Self::NoCors => "no-cors",
            Self::SameOrigin => "same-origin",
            Self::Navigate => "navigate",
            Self::Websocket => "websocket",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Site {
    #[default]
    SameOrigin,
    SameSite,
    CrossSite,
}
impl Site {
    fn name(self) -> &'static str {
        match self {
            Self::SameOrigin => "same-origin",
            Self::SameSite => "same-site",
            Self::CrossSite => "cross-site",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub mode: Option<Mode>,
    /// Fetch's enumerated destination, validated before publication.
    pub destination: Option<String>,
    pub opaque_initiator: bool,
    /// Captured at navigation acceptance, never read from later activation.
    pub user_activation: bool,
    pub redirect_taint: Site,
}
fn destination(value: &str) -> bool {
    matches!(
        value,
        "" | "audio"
            | "audioworklet"
            | "document"
            | "embed"
            | "empty"
            | "fencedframe"
            | "font"
            | "frame"
            | "iframe"
            | "image"
            | "json"
            | "manifest"
            | "object"
            | "paintworklet"
            | "report"
            | "script"
            | "serviceworker"
            | "sharedworker"
            | "style"
            | "text"
            | "track"
            | "video"
            | "webidentity"
            | "worker"
            | "xslt"
    )
}
impl Metadata {
    pub fn resource(mode: &str, destination: &str, opaque: bool) -> Self {
        Self {
            mode: Mode::parse(mode),
            destination: Some(destination.into()),
            opaque_initiator: opaque,
            ..Self::default()
        }
    }
    pub fn observe(&mut self, initiator: Option<&str>, target: &Url) {
        self.redirect_taint =
            self.redirect_taint
                .max(classify(initiator, target, self.opaque_initiator));
    }
}
fn classify(source: Option<&str>, target: &Url, opaque: bool) -> Site {
    if opaque {
        return Site::CrossSite;
    }
    let Some(source) = source.and_then(|s| Url::parse(s).ok()) else {
        return Site::CrossSite;
    };
    if !matches!(source.origin(), url::Origin::Opaque(_)) && source.origin() == target.origin() {
        return Site::SameOrigin;
    }
    // Same-origin hosts without a registrable domain are handled above. Public
    // suffixes do not become shared sites merely because both have no eTLD+1.
    if SchemefulSite::from_url(&source)
        .zip(SchemefulSite::from_url(target))
        .is_some_and(|(a, b)| a == b)
    {
        Site::SameSite
    } else {
        Site::CrossSite
    }
}
fn trustworthy(url: &Url) -> bool {
    matches!(url.scheme(), "https" | "wss")
        || matches!(url.host(),Some(url::Host::Ipv4(ip))if ip.is_loopback())
        || matches!(url.host(),Some(url::Host::Ipv6(ip))if ip.is_loopback())
        || url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host.to_ascii_lowercase().ends_with(".localhost")
        })
}
fn defaults(context: &CookieRequestContext) -> (Mode, &'static str) {
    match context.kind {
        CookieRequestKind::Document | CookieRequestKind::Form => (Mode::Navigate, "document"),
        CookieRequestKind::Frame => (Mode::Navigate, "iframe"),
        CookieRequestKind::Script => (Mode::NoCors, "script"),
        CookieRequestKind::Style => (Mode::NoCors, "style"),
        CookieRequestKind::Image => (Mode::NoCors, "image"),
        CookieRequestKind::Font => (Mode::Cors, "font"),
        CookieRequestKind::Media => (Mode::NoCors, "video"),
        CookieRequestKind::Object => (Mode::NoCors, "object"),
        CookieRequestKind::Manifest => (Mode::Cors, "manifest"),
        CookieRequestKind::Worker => (Mode::SameOrigin, "worker"),
        CookieRequestKind::WebSocket => (Mode::Websocket, "empty"),
        _ => (Mode::Cors, "empty"),
    }
}
pub(crate) fn apply(
    request: &mut crate::network::NetworkRequest,
) -> Result<(), crate::network::NetworkError> {
    request
        .headers
        .retain(|(name, _)| !name.to_ascii_lowercase().starts_with("sec-fetch-"));
    let url = Url::parse(&request.url)
        .map_err(|error| crate::network::NetworkError::InvalidUrl(error.to_string()))?;
    if !trustworthy(&url) {
        return Ok(());
    }
    let context = &request.security;
    let metadata = &context.fetch_metadata;
    let (mode, default_destination) = defaults(context);
    let mode = metadata.mode.unwrap_or(mode);
    let dest = metadata
        .destination
        .as_deref()
        .unwrap_or(default_destination);
    if !destination(dest) {
        return Err(crate::network::NetworkError::Transport(
            "invalid native Fetch destination".into(),
        ));
    }
    let site = if mode == Mode::Navigate
        && context.top_level_navigation
        && context.initiator_url.is_none()
        && !metadata.opaque_initiator
    {
        "none"
    } else {
        metadata
            .redirect_taint
            .max(classify(
                context.initiator_url.as_deref(),
                &url,
                metadata.opaque_initiator,
            ))
            .name()
    };
    request.headers.push(("sec-fetch-site".into(), site.into()));
    request
        .headers
        .push(("sec-fetch-mode".into(), mode.name().into()));
    request.headers.push((
        "sec-fetch-dest".into(),
        if dest.is_empty() { "empty" } else { dest }.into(),
    ));
    if mode == Mode::Navigate && metadata.user_activation {
        request.headers.push(("sec-fetch-user".into(), "?1".into()));
    }
    Ok(())
}
/// WebSocket handshakes use Fetch's HTTP-equivalent URL for site computation.
pub(crate) fn websocket_headers(
    url: &Url,
    origin: &str,
    opaque: bool,
) -> Vec<(&'static str, String)> {
    if !trustworthy(url) {
        return Vec::new();
    }
    let mut target = url.clone();
    let _ = target.set_scheme(if url.scheme() == "wss" {
        "https"
    } else {
        "http"
    });
    vec![
        (
            "sec-fetch-site",
            classify(Some(origin), &target, opaque).name().into(),
        ),
        ("sec-fetch-mode", "websocket".into()),
        ("sec-fetch-dest", "empty".into()),
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(source: Option<&str>, url: &str) -> crate::network::NetworkRequest {
        crate::network::NetworkRequest {
            request_id: 1,
            method: "GET".into(),
            url: url.into(),
            headers: vec![
                ("Sec-Fetch-Site".into(), "none".into()),
                ("sec-fetch-user".into(), "?1".into()),
            ],
            body: vec![],
            credentials: false,
            resolved_addrs: vec![],
            security: CookieRequestContext {
                initiator_url: source.map(str::to_owned),
                kind: CookieRequestKind::Media,
                fetch_metadata: Metadata::resource("no-cors", "audio", false),
                ..Default::default()
            },
        }
    }
    fn header(r: &crate::network::NetworkRequest, name: &str) -> Option<String> {
        r.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    }
    #[test]
    fn native_media_metadata_overwrites_authored_values_and_obeys_secure_delivery() {
        for (source, target, site) in [
            (
                "https://a.example.test",
                "https://a.example.test/x",
                "same-origin",
            ),
            (
                "https://a.example.test",
                "https://b.example.test/x",
                "same-site",
            ),
            (
                "http://a.example.test",
                "https://a.example.test/x",
                "cross-site",
            ),
            ("https://example.test", "https://other.test/x", "cross-site"),
        ] {
            let mut r = request(Some(source), target);
            apply(&mut r).unwrap();
            assert_eq!(header(&r, "sec-fetch-site").as_deref(), Some(site));
            assert_eq!(header(&r, "sec-fetch-dest").as_deref(), Some("audio"));
            assert_eq!(header(&r, "sec-fetch-user"), None);
        }
        let mut r = request(Some("http://a.test"), "http://a.test/x");
        apply(&mut r).unwrap();
        assert!(r.headers.is_empty());
    }
    #[test]
    fn redirect_site_is_monotonic_and_distinct_from_cookie_ancestry() {
        let mut r = request(
            Some("https://a.example.test"),
            "https://a.example.test/final",
        );
        r.security.fetch_metadata.observe(
            r.security.initiator_url.as_deref(),
            &Url::parse("https://other.test/redirect").unwrap(),
        );
        apply(&mut r).unwrap();
        assert_eq!(header(&r, "sec-fetch-site").as_deref(), Some("cross-site"));
        let mut r = request(
            Some("https://a.example.test"),
            "https://a.example.test/final",
        );
        r.security.fetch_metadata.observe(
            r.security.initiator_url.as_deref(),
            &Url::parse("https://b.example.test/redirect").unwrap(),
        );
        apply(&mut r).unwrap();
        assert_eq!(header(&r, "sec-fetch-site").as_deref(), Some("same-site"));
    }
    #[test]
    fn navigation_none_and_user_activation_require_native_navigation_provenance() {
        let mut r = request(None, "https://target.test/");
        r.security = CookieRequestContext::navigation(None);
        r.security.fetch_metadata.user_activation = true;
        apply(&mut r).unwrap();
        assert_eq!(header(&r, "sec-fetch-site").as_deref(), Some("none"));
        assert_eq!(header(&r, "sec-fetch-user").as_deref(), Some("?1"));
        r.security.fetch_metadata.opaque_initiator = true;
        apply(&mut r).unwrap();
        assert_eq!(header(&r, "sec-fetch-site").as_deref(), Some("cross-site"));
        r.security.fetch_metadata.mode = Some(Mode::Cors);
        apply(&mut r).unwrap();
        assert_eq!(header(&r, "sec-fetch-user"), None);
    }
}

#[cfg(test)]
mod handshake_tests {
    use super::*;
    #[test]
    fn websocket_site_uses_http_equivalent_origin_and_never_activation() {
        let headers = websocket_headers(
            &Url::parse("wss://source.test/socket").unwrap(),
            "https://source.test",
            false,
        );
        assert_eq!(
            headers,
            vec![
                ("sec-fetch-site", "same-origin".into()),
                ("sec-fetch-mode", "websocket".into()),
                ("sec-fetch-dest", "empty".into())
            ]
        );
        assert_eq!(
            websocket_headers(
                &Url::parse("wss://source.test/socket").unwrap(),
                "https://source.test",
                true
            )[0]
            .1,
            "cross-site"
        );
        assert!(
            websocket_headers(
                &Url::parse("ws://source.test/socket").unwrap(),
                "http://source.test",
                false
            )
            .is_empty()
        );
    }
}
