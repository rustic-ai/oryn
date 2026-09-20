//! Route-owned initiating authority, independent of destination response policy.
//! All operations are serialized by RouteAuthority. Context validation and broker
//! callbacks run outside that lock. Closed nonces are never evicted or reused.
use crate::{
    context_service::NavigationNetworkAuthority,
    cookies::{CookieRequestKind, SchemefulSite},
    network::{NetworkError, NetworkRequest},
    referrer::ReferrerPolicy,
};
use oryn_js::{navigation::FormNavigationSource, worker_wire::Creator};
use std::collections::{BTreeMap, BTreeSet};
use url::Url;
const ACTIVE_LIMIT: usize = 64;
// Fail closed when replay history is full; replacement routes get fresh custody.
const NONCE_LIMIT: usize = 4096;
fn invalid(message: &str) -> NetworkError {
    NetworkError::Transport(message.into())
}
fn valid_nonce(nonce: &str) -> bool {
    nonce.len() == 64 && nonce.bytes().all(|b| b.is_ascii_hexdigit())
}
#[derive(Clone)]
pub(super) struct Seal {
    pub authority: NavigationNetworkAuthority,
    pub target: Creator,
    pub initial_url: String,
    pub service_get: bool,
}
enum Leg {
    Ready(String),
    Running { id: u64, url: String },
    Finished,
}
struct Grant {
    seal: Seal,
    source: FormNavigationSource,
    leg: Leg,
    used: BTreeSet<u64>,
    redirects: usize,
    max_redirects: usize,
    referrer: ReferrerPolicy,
    referrer_source: Option<Url>,
    taint: crate::fetch_metadata::Site,
    cross_site: bool,
}
#[derive(Default)]
pub(super) struct Registry {
    retired: bool,
    // None is an acknowledged cancellation/completion tombstone. It prevents
    // a delayed acquire from publishing after its prearmed cleanup has won.
    grants: BTreeMap<String, Option<Grant>>,
}
impl Registry {
    pub fn begin(
        &mut self,
        nonce: String,
        seal: Seal,
        source: FormNavigationSource,
        max_redirects: usize,
    ) -> Result<(), NetworkError> {
        if self.retired || !valid_nonce(&nonce) || self.grants.contains_key(&nonce) {
            return Err(invalid("navigation source nonce retired or reused"));
        }
        if self.grants.len() >= NONCE_LIMIT
            || self.grants.values().filter(|v| v.is_some()).count() >= ACTIVE_LIMIT
        {
            return Err(invalid("navigation source custody capacity exceeded"));
        }
        let url = Url::parse(&seal.initial_url)
            .map_err(|_| invalid("invalid navigation source URL"))?
            .to_string();
        self.grants.insert(
            nonce,
            Some(Grant {
                referrer: ReferrerPolicy::parse_list(&source.referrer_policy),
                referrer_source: (!source.opaque_origin)
                    .then(|| Url::parse(&source.url).ok())
                    .flatten(),
                source,
                seal,
                leg: Leg::Ready(url),
                used: BTreeSet::new(),
                redirects: 0,
                max_redirects,
                taint: Default::default(),
                cross_site: false,
            }),
        );
        Ok(())
    }
    pub fn seal(&self, nonce: &str) -> Result<Seal, NetworkError> {
        if self.retired {
            return Err(invalid("navigation source route retired"));
        }
        self.grants
            .get(nonce)
            .and_then(Option::as_ref)
            .map(|g| g.seal.clone())
            .ok_or_else(|| invalid("unknown navigation source lease"))
    }
    pub fn consume(
        &mut self,
        nonce: &str,
        requested_url: &str,
        request: &mut NetworkRequest,
    ) -> Result<(), NetworkError> {
        if self.retired {
            return Err(invalid("navigation source route retired"));
        }
        let grant = self
            .grants
            .get_mut(nonce)
            .and_then(Option::as_mut)
            .ok_or_else(|| invalid("unknown navigation source lease"))?;
        let url = Url::parse(&request.url).map_err(|_| invalid("invalid source request URL"))?;
        if request.security.kind != CookieRequestKind::Document
            || (grant.seal.service_get && (request.method != "GET" || !request.body.is_empty()))
            || !matches!(&grant.leg, Leg::Ready(expected) if expected == url.as_str() || Url::parse(requested_url).ok().as_ref().is_some_and(|url| expected == url.as_str()))
            || grant.used.contains(&request.request_id)
        {
            return Err(invalid(
                "navigation source request does not own the next leg",
            ));
        }
        grant.used.insert(request.request_id);
        grant.leg = Leg::Running {
            id: request.request_id,
            url: url.to_string(),
        };
        let initiator = if grant.source.opaque_origin {
            "about:blank"
        } else {
            &grant.source.url
        };
        request.security.initiator_url = Some(initiator.into());
        request.security.top_level_url = Some(grant.source.top_level_url.clone());
        request.security.ancestor_urls = grant.source.ancestor_urls.clone();
        request.security.top_level_navigation = true;
        request.security.fetch_metadata = crate::fetch_metadata::Metadata {
            mode: Some(crate::fetch_metadata::Mode::Navigate),
            destination: Some("document".into()),
            opaque_initiator: grant.source.opaque_origin,
            user_activation: grant.source.user_activation,
            redirect_taint: grant.taint,
        };
        request
            .security
            .fetch_metadata
            .observe(Some(initiator), &url);
        grant.taint = request.security.fetch_metadata.redirect_taint;
        request.security.redirect_chain_cross_site = grant.cross_site;
        request
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("referer"));
        let referrer = grant
            .referrer_source
            .as_ref()
            .and_then(|source| grant.referrer.header(source, &url));
        grant.referrer_source = referrer.as_deref().and_then(|value| Url::parse(value).ok());
        if let Some(value) = referrer {
            request.headers.push(("referer".into(), value));
        }
        Ok(())
    }
    pub fn headers(
        &mut self,
        nonce: &str,
        id: u64,
        status: u16,
        headers: &[(String, String)],
    ) -> Result<(), NetworkError> {
        if self.retired {
            return Err(invalid("navigation source route retired"));
        }
        let grant = self
            .grants
            .get_mut(nonce)
            .and_then(Option::as_mut)
            .ok_or_else(|| invalid("navigation source lease retired"))?;
        let Leg::Running { id: owned, url } = &grant.leg else {
            return Err(invalid("duplicate navigation source headers"));
        };
        if *owned != id {
            return Err(invalid("foreign navigation source response"));
        }
        let current = Url::parse(url).map_err(|_| invalid("invalid retained navigation URL"))?;
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            let next = headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                .and_then(|(_, value)| current.join(value).ok())
                .filter(|url| matches!(url.scheme(), "http" | "https"));
            if let Some(next) = next {
                if grant.redirects >= grant.max_redirects {
                    grant.leg = Leg::Finished;
                    return Err(invalid("navigation source redirect limit exceeded"));
                }
                grant.redirects += 1;
                grant.cross_site |= SchemefulSite::from_url(&next)
                    != Url::parse(&grant.seal.initial_url)
                        .ok()
                        .as_ref()
                        .and_then(SchemefulSite::from_url);
                grant.referrer = grant.referrer.after_response(headers);
                grant.leg = Leg::Ready(next.to_string());
                return Ok(());
            }
        }
        grant.leg = Leg::Finished;
        Ok(())
    }
    // Dropping an old redirect response does not cancel the authorized next leg.
    pub fn finish_request(&mut self, nonce: &str, id: u64) {
        if let Some(Some(grant)) = self.grants.get_mut(nonce)
            && matches!(grant.leg,Leg::Running{id:owned,..} if owned==id)
        {
            grant.leg = Leg::Finished;
        }
    }
    pub fn cancel_request(&mut self, id: u64) {
        for grant in self.grants.values_mut().flatten() {
            if matches!(grant.leg,Leg::Running{id:owned,..} if owned==id) {
                grant.leg = Leg::Finished;
            }
        }
    }
    pub fn discard(&mut self, nonce: &str) {
        if self.retired || !valid_nonce(nonce) {
            return;
        }
        if let Some(value) = self.grants.get_mut(nonce) {
            *value = None;
        } else if self.grants.len() < NONCE_LIMIT {
            self.grants.insert(nonce.into(), None);
        } else {
            self.revoke();
        } // never lose a cancellation tombstone
    }
    pub fn revoke(&mut self) {
        self.retired = true;
        self.grants.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cookies::CookieRequestContext;
    fn nonce(n: u64) -> String {
        format!("{n:064x}")
    }
    fn seal() -> Seal {
        Seal {
            authority: NavigationNetworkAuthority::Service(
                oryn_js::navigation::ServiceNavigationAuthority(vec![]),
            ),
            target: Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 1,
            },
            initial_url: "https://initial.test/start".into(),
            service_get: true,
        }
    }
    // Registry tests begin after authentication; broker integration tests exercise the seal.
    fn source() -> FormNavigationSource {
        FormNavigationSource {
            user_activation: false,
            source_domain_key: None,
            origin_key: "https://service.test".into(),
            url: "https://service.test/sw.js".into(),
            referrer_policy: "no-referrer".into(),
            top_level_url: "https://top.test/".into(),
            ancestor_urls: vec![
                "https://service.test/sw.js".into(),
                "https://top.test/".into(),
            ],
            opaque_origin: false,
        }
    }
    fn request(id: u64, url: &str) -> NetworkRequest {
        NetworkRequest {
            request_id: id,
            method: "GET".into(),
            url: url.into(),
            headers: vec![("referer".into(), "https://forged.test/".into())],
            body: vec![],
            credentials: true,
            security: CookieRequestContext {
                kind: CookieRequestKind::Document,
                initiator_url: Some("https://forged.test/".into()),
                top_level_url: Some("https://forged.test/".into()),
                ancestor_urls: vec!["https://forged.test/".into()],
                ..Default::default()
            },
            resolved_addrs: vec![],
        }
    }
    fn registry() -> Registry {
        let mut registry = Registry::default();
        registry.begin(nonce(1), seal(), source(), 2).unwrap();
        registry
    }
    fn consume(
        registry: &mut Registry,
        token: u64,
        id: u64,
        url: &str,
    ) -> Result<NetworkRequest, NetworkError> {
        let mut request = request(id, url);
        registry.consume(&nonce(token), url, &mut request)?;
        Ok(request)
    }
    #[test]
    fn nonce_owns_exact_document_request_and_ignores_child_source_metadata() {
        let mut registry = registry();
        let url = seal().initial_url;
        assert!(consume(&mut registry, 2, 1, &url).is_err());
        assert!(consume(&mut registry, 1, 1, "https://unrelated.test/").is_err());
        let mut script = request(1, &url);
        script.security.kind = CookieRequestKind::Script;
        assert!(registry.consume(&nonce(1), &url, &mut script).is_err());
        let mut post = request(1, &url);
        post.method = "POST".into();
        assert!(registry.consume(&nonce(1), &url, &mut post).is_err());
        post.method = "GET".into();
        post.body = vec![1];
        assert!(registry.consume(&nonce(1), &url, &mut post).is_err());
        let request = consume(&mut registry, 1, 1, &url).unwrap();
        assert_eq!(request.security.initiator_url, Some(source().url));
        assert_eq!(request.security.top_level_url, Some(source().top_level_url));
        assert_eq!(request.security.ancestor_urls, source().ancestor_urls);
        assert!(!request.security.fetch_metadata.user_activation);
        assert!(request.security.top_level_navigation);
        assert!(
            !request
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("referer"))
        );
        assert!(consume(&mut registry, 1, 2, &url).is_err());
        assert!(registry.begin(nonce(1), seal(), source(), 2).is_err());
        // A concurrent independent lease for the same URL cannot steal this leg.
        registry.begin(nonce(2), seal(), source(), 2).unwrap();
        assert!(consume(&mut registry, 2, 3, &url).is_ok());
    }
    #[test]
    fn only_trusted_redirect_headers_advance_and_old_cleanup_cannot_cancel_new_leg() {
        let mut registry = registry();
        let url = seal().initial_url;
        consume(&mut registry, 1, 7, &url).unwrap();
        assert!(
            registry
                .headers(&nonce(1), 8, 302, &[("location".into(), "/next".into())])
                .is_err()
        );
        registry
            .headers(
                &nonce(1),
                7,
                302,
                &[("location".into(), "https://final.test/receipt".into())],
            )
            .unwrap();
        registry.finish_request(&nonce(1), 7);
        registry.cancel_request(7);
        assert!(consume(&mut registry, 1, 8, &url).is_err());
        let final_request = consume(&mut registry, 1, 8, "https://final.test/receipt").unwrap();
        assert!(final_request.security.redirect_chain_cross_site);
        registry.finish_request(&nonce(1), 7);
        registry.cancel_request(7);
        registry
            .headers(
                &nonce(1),
                8,
                200,
                &[("location".into(), "https://forged.test/".into())],
            )
            .unwrap();
        assert!(consume(&mut registry, 1, 9, "https://forged.test/").is_err());
        assert!(consume(&mut registry, 1, 9, &url).is_err());
    }
    #[test]
    fn relative_redirects_limits_request_id_reuse_and_no_referrer_revival() {
        let mut registry = registry();
        let url = seal().initial_url;
        consume(&mut registry, 1, 1, &url).unwrap();
        registry
            .headers(
                &nonce(1),
                1,
                307,
                &[
                    ("location".into(), "../next".into()),
                    ("referrer-policy".into(), "unsafe-url".into()),
                ],
            )
            .unwrap();
        assert!(consume(&mut registry, 1, 1, "https://initial.test/next").is_err());
        let request = consume(&mut registry, 1, 2, "https://initial.test/next").unwrap();
        assert!(
            !request
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("referer"))
        );
        registry
            .headers(&nonce(1), 2, 308, &[("location".into(), "/last".into())])
            .unwrap();
        consume(&mut registry, 1, 3, "https://initial.test/last").unwrap();
        assert!(
            registry
                .headers(&nonce(1), 3, 303, &[("location".into(), "/too-far".into())])
                .is_err()
        );
        assert!(consume(&mut registry, 1, 4, "https://initial.test/too-far").is_err());
    }
    #[test]
    fn parent_url_upgrades_use_effective_redirect_base_and_referrer_reduction_is_monotonic() {
        let mut registry = Registry::default();
        let mut initial = seal();
        initial.initial_url = "http://initial.test/start".into();
        let mut metadata = source();
        metadata.referrer_policy = "origin".into();
        registry.begin(nonce(1), initial, metadata, 2).unwrap();
        let mut first = request(1, "https://initial.test/start");
        registry
            .consume(&nonce(1), "http://initial.test/start", &mut first)
            .unwrap();
        assert_eq!(
            first
                .headers
                .iter()
                .find(|(name, _)| name == "referer")
                .map(|(_, value)| value.as_str()),
            Some("https://service.test/")
        );
        registry
            .headers(
                &nonce(1),
                1,
                302,
                &[
                    ("location".into(), "/next".into()),
                    ("referrer-policy".into(), "unsafe-url".into()),
                ],
            )
            .unwrap();
        let mut second = request(2, "https://initial.test/next");
        registry
            .consume(&nonce(1), "http://initial.test/next", &mut second)
            .unwrap();
        assert_eq!(
            second
                .headers
                .iter()
                .find(|(name, _)| name == "referer")
                .map(|(_, value)| value.as_str()),
            Some("https://service.test/")
        );
    }

    #[test]
    fn cancellation_before_publication_and_after_redirect_never_resurrects_authority() {
        let mut registry = Registry::default();
        registry.discard(&nonce(1));
        assert!(registry.begin(nonce(1), seal(), source(), 2).is_err());
        registry.begin(nonce(2), seal(), source(), 2).unwrap();
        consume(&mut registry, 2, 1, &seal().initial_url).unwrap();
        registry
            .headers(&nonce(2), 1, 302, &[("location".into(), "/next".into())])
            .unwrap();
        registry.discard(&nonce(2));
        assert!(consume(&mut registry, 2, 2, "https://initial.test/next").is_err());
        assert!(registry.begin(nonce(2), seal(), source(), 2).is_err());
    }
    #[test]
    fn failed_start_duplicate_headers_and_route_revocation_are_terminal() {
        let mut registry = registry();
        let url = seal().initial_url;
        consume(&mut registry, 1, 1, &url).unwrap();
        registry.finish_request(&nonce(1), 1);
        assert!(consume(&mut registry, 1, 2, &url).is_err());
        assert!(
            registry
                .headers(&nonce(1), 1, 302, &[("location".into(), "/next".into())])
                .is_err()
        );
        registry.revoke();
        assert!(registry.begin(nonce(2), seal(), source(), 2).is_err());
        assert!(registry.seal(&nonce(1)).is_err());
    }
    #[test]
    fn bounded_cleanup_history_never_evicts_a_nonce_to_allow_late_acquisition() {
        let mut registry = Registry::default();
        for id in 1..=NONCE_LIMIT as u64 {
            registry.discard(&nonce(id));
        }
        assert_eq!(registry.grants.len(), NONCE_LIMIT);
        assert!(registry.begin(nonce(1), seal(), source(), 2).is_err());
        registry.discard(&nonce(NONCE_LIMIT as u64 + 1));
        assert!(registry.retired);
        assert!(
            registry
                .begin(nonce(NONCE_LIMIT as u64 + 2), seal(), source(), 2)
                .is_err()
        );
    }
}
