//! Broker-backed frame responses owned by the existing native frame task.
//! Only the bounded resource IO executor calls this loader.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Loader {
    kind: oryn_dom::EmbeddedContentKind,
    declared_type: Option<String>,
    pub(super) local_blob: Option<crate::standalone_document_policy::BlobResource>,
    pub(super) request_state: Option<oryn_common::navigation_request::NavigationRequestState>,
    user_activation: bool,
    parent_encoding: String,
    pub(super) navigation: oryn_js::platform::NavigationRequestFlags,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) broker: SharedNetworkBroker,
    network_broker_available: bool,
    document_policy: crate::standalone_document_policy::DocumentPolicy,
    isolation: crate::agent_isolation::DocumentIsolationPolicy,
    container_allow: String,
    policy: Option<crate::csp::CspPolicySet>,
    reports: Arc<module_graph_load::ScriptCspReports>,
    ancestors: Vec<String>,
    initiator: Url,
    top: Url,
    next: u64,
}
impl Loader {
    /// Local decoding may use an unavailable transport as an execution handle.
    /// It is never authority to install a new network endpoint in the child.
    pub(super) fn committed_broker(&self) -> Option<SharedNetworkBroker> {
        self.network_broker_available.then(|| self.broker.clone())
    }

    pub(super) fn retained_bytes(&self) -> Result<usize, String> {
        let mut size = crate::retained_size::Size(std::mem::size_of::<Self>());
        size.string(&self.parent_encoding)?;
        size.optional(&self.declared_type)?;
        size.string(&self.container_allow)?;
        size.vector(&self.ancestors)?;
        for value in &self.ancestors {
            size.string(value)?;
        }
        // url::Url does not expose capacity. Its owned serialization is bounded
        // conservatively here; the original request String is charged separately.
        size.add(
            self.initiator
                .as_str()
                .len()
                .checked_mul(2)
                .ok_or("frame URL bytes overflow")?,
        )?;
        size.add(
            self.top
                .as_str()
                .len()
                .checked_mul(2)
                .ok_or("frame URL bytes overflow")?,
        )?;
        size.add(self.document_policy.retained_bytes()?)?;
        if let Some(policy) = &self.policy {
            size.add(policy.retained_bytes()?)?;
        }
        if let Some(state) = &self.request_state {
            state.retained_bytes().map_err(str::to_owned)?;
            size.optional(&state.initiator_url)?;
            size.optional(&state.referrer)?;
            size.string(&state.referrer_policy)?;
        }
        if let Some(blob) = &self.local_blob {
            size.vector(&blob.bytes)?;
            size.string(&blob.mime)?;
            size.add(
                blob.origin
                    .retained_bytes()
                    .ok_or("frame blob origin bytes overflow")?,
            )?;
            size.add(blob.document.retained_bytes()?)?;
            if let Some(policy) = &blob.csp {
                size.add(policy.retained_bytes()?)?;
            }
        }
        // broker/reports are shared handles whose backing stores have separate
        // bounded owners; no new backing allocation is cloned by this loader.
        Ok(size.0)
    }

    pub(super) fn select_document_navigation(&mut self) {
        self.kind = oryn_dom::EmbeddedContentKind::Frame;
        self.declared_type = None;
    }

    #[cfg(test)]
    pub(super) fn capture(
        page: &mut RustExecutableDocument,
        parent: RealmId,
        frame: NodeId,
    ) -> Result<Self, HostError> {
        Self::capture_for_url(page, parent, frame, None)
    }
    pub(super) fn capture_for_url(
        page: &mut RustExecutableDocument,
        parent: RealmId,
        frame: NodeId,
        url: Option<&str>,
    ) -> Result<Self, HostError> {
        let local = url
            .and_then(|url| Url::parse(url).ok())
            .is_some_and(|url| matches!(url.scheme(), "data" | "blob"));
        let parent_broker = page.agent.host.network_broker_for_realm(parent);
        let local_only = parent_broker.is_none() && local;
        let broker = parent_broker
            .or_else(|| local.then(|| SharedNetworkBroker::new(crate::network::UnavailableBroker)))
            .ok_or_else(|| {
                HostError::Execution("standalone frame parent broker unavailable".into())
            })?
            .new_request_scope();
        page.agent.host.activate_realm(parent)?;
        let initiator = Url::parse(&page.agent.host.request_initiator_url()?)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let top = page.base_url.clone().unwrap_or_else(|| initiator.clone());
        let parent_dom = page
            .agent
            .host
            .automation_dom(parent)
            .ok_or_else(|| HostError::Execution("standalone frame parent DOM absent".into()))?;
        let kind = parent_dom
            .borrow()
            .embedded_content_kind(frame)
            .ok_or_else(|| HostError::Execution("invalid embedded resource owner".into()))?;
        let declared_type = parent_dom
            .borrow()
            .attribute(frame, "type")
            .ok()
            .flatten()
            .map(str::to_owned);
        let parent_encoding = document_encoding(&parent_dom);
        let document_policy = page.agent.host.standalone_document_policy()?;
        let isolation = page.agent.host.script_isolation_policy();
        let container_allow = page
            .agent
            .host
            .automation_dom(parent)
            .ok_or_else(|| HostError::Execution("standalone frame parent DOM absent".into()))?
            .borrow()
            .frame_permissions_policy_allow(frame)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let mut ancestors = Vec::new();
        let mut current = Some(parent);
        let mut seen = BTreeSet::new();
        while let Some(realm) = current {
            if !seen.insert(realm) || seen.len() > 32 {
                return Err(HostError::Execution(
                    "standalone frame ancestry cycle".into(),
                ));
            }
            page.agent.host.activate_realm(realm)?;
            ancestors.push(page.agent.host.request_initiator_url()?);
            current = page
                .frame_realms
                .get(&realm)
                .map(|owner| owner.parent_realm);
        }
        page.agent.host.activate_realm(parent)?;
        Ok(Self {
            kind,
            declared_type,
            local_blob: None,
            request_state: None,
            parent_encoding,
            user_activation: page.agent.host.navigation_user_activation()?,
            navigation: Default::default(),
            broker,
            network_broker_available: !local_only,
            document_policy,
            isolation,
            container_allow,
            policy: page.agent.host.standalone_resource_policy()?,
            reports: page.agent.host.script_report_queue()?,
            // A brokerless local response performs no IO. Its synthetic ID belongs
            // solely to this fresh request scope; HTTP retains the host allocator.
            ancestors,
            initiator,
            top,
            next: if local_only {
                0
            } else {
                page.agent.host.reserve_module_request_id()?
            },
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }
    pub(super) fn load(
        mut self,
        requested: String,
        referrer: Option<String>,
        mut post: Option<oryn_js::navigation::PostResource>,
    ) -> Result<
        (
            crate::frame_resource_response::FrameResourceResponse,
            crate::standalone_document_policy::FramePolicy,
        ),
        String,
    > {
        use crate::cookies::{CookieCredentials, CookieRequestContext, CookieRequestKind};
        use crate::frame_resource_response::FrameResourceResponse;
        let mut url = self
            .initiator
            .join(&requested)
            .map_err(|error| error.to_string())?;
        let mut _timing = self.reports.time_resource(
            url.as_str(),
            if self.kind.selects_response_representation() {
                self.kind.resource_destination()
            } else {
                "iframe"
            },
        );
        let timing_started = std::time::Instant::now();
        let mut timing_hops = Vec::new();
        let mut navigation_phases = crate::network::NavigationPhaseClock::new(timing_started);
        let container_source = url.to_string();
        let mut policy = crate::referrer::ReferrerPolicy::parse_list(
            referrer
                .as_deref()
                .unwrap_or("strict-origin-when-cross-origin"),
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut crossed = false;
        let mut referrer_source = Some(self.initiator.clone());
        let initial_site = crate::cookies::SchemefulSite::from_url(&url);
        let mut fetch_metadata = crate::fetch_metadata::Metadata::resource(
            "navigate",
            if self.kind.selects_response_representation() {
                self.kind.resource_destination()
            } else {
                "iframe"
            },
            self.document_policy.origin == "null",
        );
        let request_initiator = self.request_state.as_ref().map_or_else(
            || Some(self.initiator.to_string()),
            |state| state.initiator_url.clone(),
        );
        let request_opaque = self
            .request_state
            .as_ref()
            .map_or(self.document_policy.origin == "null", |state| {
                state.opaque_initiator
            });
        if let Some(state) = &self.request_state {
            state.retained_bytes().map_err(str::to_owned)?;
            if !(self.navigation.reload || self.navigation.history) {
                return Err("request provenance requires frame history navigation".into());
            }
            referrer_source = state
                .referrer
                .as_deref()
                .map(Url::parse)
                .transpose()
                .map_err(|error| error.to_string())?;
            policy = crate::referrer::ReferrerPolicy::parse_list(&state.referrer_policy);
        }
        let request_origin = request_initiator
            .as_deref()
            .and_then(|url| Url::parse(url).ok())
            .map(|url| url.origin().ascii_serialization());
        let origin_source = request_initiator
            .as_deref()
            .and_then(|value| Url::parse(value).ok())
            .unwrap_or_else(|| self.initiator.clone());
        let mut navigation_origin = if request_opaque {
            "null".into()
        } else {
            request_origin.clone().unwrap_or_else(|| "null".into())
        };
        fetch_metadata.opaque_initiator = request_opaque;
        fetch_metadata.user_activation = self.user_activation;
        for redirect in 0..=10 {
            navigation_phases.begin_hop();
            if self.cancel.load(Ordering::Acquire) {
                return Err("standalone frame cancelled".into());
            }
            if let Some(csp) = &self.policy {
                if let Some(upgraded) = csp.upgraded_url(&url) {
                    url = upgraded;
                }
                let decision = csp.authorize_redirect_from(
                    &crate::csp::CspRequest {
                        destination: if self.kind.selects_response_representation() {
                            crate::csp::CspDestination::Object
                        } else {
                            crate::csp::CspDestination::Frame
                        },
                        url: url.to_string(),
                        nonce: None,
                        inline_source: None,
                        parser_inserted: true,
                        initiated_by_trusted_script: false,
                    },
                    redirect != 0,
                    &container_source,
                );
                self.reports.push_with_policy(csp, decision.violations)?;
                if !decision.allowed {
                    return Err("frame blocked by captured Content-Security-Policy".into());
                }
            }
            if self.cancel.load(Ordering::Acquire) {
                return Err("standalone frame cancelled after policy reporting".into());
            }
            crate::mixed_content::enforce(self.initiator.as_str(), &url)
                .map_err(|error| error.to_string())?;
            let response = if url.scheme() == "data" {
                if post.is_some() {
                    return Err("frame POST requires HTTP".into());
                }
                let (mime, body) =
                    crate::network::decode_data_url(&url).map_err(|error| error.to_string())?;
                crate::network::NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: self.next,
                    status: 200,
                    headers: mime
                        .into_iter()
                        .map(|mime| ("content-type".into(), mime))
                        .collect(),
                    body,
                }
            } else if url.scheme() == "blob" {
                if post.is_some() {
                    return Err("blob navigation requires GET".into());
                }
                let blob = self
                    .local_blob
                    .as_ref()
                    .ok_or("standalone blob response unavailable or revoked")?;
                crate::network::NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: self.next,
                    status: 200,
                    headers: vec![("content-type".into(), blob.mime.clone())],
                    body: blob.bytes.clone(),
                }
            } else {
                if !matches!(url.scheme(), "http" | "https") {
                    return Err("standalone frame URL has no registered loader".into());
                }
                if post.as_ref().is_some_and(|post| !post.valid_for(&url)) {
                    return Err("invalid standalone frame POST".into());
                }
                let mut headers = vec![(
                    "accept".into(),
                    "text/html,application/xhtml+xml;q=0.9,*/*;q=0.1".into(),
                )];
                let sent_referrer = referrer_source
                    .as_ref()
                    .and_then(|source| policy.header(source, &url));
                referrer_source = sent_referrer
                    .as_ref()
                    .and_then(|value| Url::parse(value).ok());
                if let Some(value) = sent_referrer {
                    headers.push(("referer".into(), value));
                }
                if let Some(post) = &post {
                    headers.push(("content-type".into(), post.content_type.clone()));
                }
                if let Some(value) = crate::referrer::request_origin_header(
                    &navigation_origin,
                    &origin_source,
                    &url,
                    if post.is_some() { "POST" } else { "GET" },
                    false,
                    policy,
                ) {
                    headers.push(("origin".into(), value));
                }
                // Credentialless only strips no-cors subresources; this is navigate.
                let omit = false;
                fetch_metadata.observe(request_origin.as_deref(), &url);
                let request_id = self.next;
                self.next = self
                    .next
                    .checked_add(1)
                    .ok_or("standalone frame request identity exhausted")?;
                crate::network::collect_operation_bounded_with_cancel(
                    &self.broker,
                    crate::network::NetworkRequest {
                        request_id,
                        method: if post.is_some() { "POST" } else { "GET" }.into(),
                        url: url.to_string(),
                        headers,
                        body: post
                            .as_ref()
                            .map_or_else(Vec::new, |post| post.bytes.clone()),
                        credentials: !omit,
                        resolved_addrs: Vec::new(),
                        security: CookieRequestContext {
                            fetch_metadata: fetch_metadata.clone(),
                            dns_timing: None,
                            navigation: self.navigation,
                            credentials: if omit {
                                CookieCredentials::Omit
                            } else {
                                CookieCredentials::Include
                            },
                            kind: if self.kind.selects_response_representation() {
                                CookieRequestKind::Object
                            } else {
                                CookieRequestKind::Frame
                            },
                            initiator_url: if request_opaque {
                                Some("about:blank".into())
                            } else {
                                request_initiator.clone()
                            },
                            top_level_url: Some(self.top.to_string()),
                            ancestor_urls: self.ancestors.clone(),
                            redirect_chain_cross_site: crossed,
                            ..Default::default()
                        },
                    },
                    deadline
                        .checked_duration_since(std::time::Instant::now())
                        .ok_or("standalone frame deadline exceeded")?,
                    16 * 1024 * 1024,
                    &self.cancel,
                )
                .map_err(|error| error.to_string())?
            };
            if self.cancel.load(Ordering::Acquire) {
                return Err("standalone frame cancelled before publication".into());
            }
            timing_hops.push(crate::network::timing_hop(&url, &response.headers));
            if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                navigation_phases.redirected();
                if redirect == 10 {
                    return Err("standalone frame redirect limit".into());
                }
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or("frame redirect missing Location")?;
                let next = url.join(location).map_err(|error| error.to_string())?;
                crossed |= crate::cookies::SchemefulSite::from_url(&next) != initial_site;
                if url.origin() != next.origin() && url.origin() != origin_source.origin() {
                    navigation_origin = "null".into();
                }
                oryn_js::navigation::PostResource::redirect(&mut post, response.status);
                policy = policy.after_response(&response.headers);
                url = next;
                continue;
            }
            let response_policy = if url.scheme() == "blob" {
                self.local_blob
                    .as_ref()
                    .and_then(|blob| blob.csp.clone())
                    .unwrap_or_else(|| crate::csp::CspPolicySet::from_headers(url.as_str(), &[]))
            } else if url.scheme() == "data" {
                self.policy
                    .clone()
                    .unwrap_or_else(|| crate::csp::CspPolicySet::from_headers(url.as_str(), &[]))
            } else {
                crate::csp::CspPolicySet::from_headers(url.as_str(), &response.headers)
            };
            if matches!(response.status, 204 | 205) {
                return Ok((
                    FrameResourceResponse::PageResourceNoCommit {
                        reason: "no-content".into(),
                    },
                    crate::standalone_document_policy::FramePolicy {
                        csp: response_policy,
                        document: self.document_policy.clone(),
                    },
                ));
            }
            if !self.kind.selects_response_representation()
                && crate::download::response_is_download(&response.headers)
            {
                return Ok((
                    FrameResourceResponse::PageResourceNoCommit {
                        reason: "download requires parent quarantine".into(),
                    },
                    crate::standalone_document_policy::FramePolicy {
                        csp: response_policy,
                        document: self.document_policy.clone(),
                    },
                ));
            }
            if response.body.len() > 16 * 1024 * 1024 {
                return Err("standalone frame response bytes exceeded".into());
            }
            let supplied_type = oryn_common::mime_type::MimeType::from_headers(&response.headers)
                .map(|mime| mime.serialize());
            let no_sniff = crate::network::response_no_sniff(&response.headers);
            let sniff_type = crate::embedded_resource_type::sniff_input(
                self.kind.resource_destination(),
                supplied_type.as_deref(),
            );
            let computed = oryn_net::mime_sniff::document_type(
                sniff_type.as_deref(),
                no_sniff,
                &response.body,
            );
            let content_type = Some(if self.kind.selects_response_representation() {
                match crate::embedded_resource_type::select(
                    self.kind.resource_destination(),
                    supplied_type.as_deref(),
                    self.declared_type.as_deref(),
                    &computed,
                    no_sniff,
                ) {
                    Ok(value) => value,
                    Err(_) => {
                        return Ok((
                            FrameResourceResponse::EmbeddedResourceUnavailable,
                            crate::standalone_document_policy::FramePolicy {
                                csp: self.policy.clone().unwrap_or_else(|| {
                                    crate::csp::CspPolicySet::from_headers(url.as_str(), &[])
                                }),
                                document: self.document_policy.clone(),
                            },
                        ));
                    }
                }
            } else {
                computed
            });
            let child_document = !self.kind.selects_response_representation()
                || crate::frame_resource_response::embedded_document_mime(content_type.as_deref());
            if self.kind == oryn_dom::EmbeddedContentKind::Object
                && !(200..300).contains(&response.status)
            {
                return Err(format!(
                    "embedded response returned HTTP {}",
                    response.status
                ));
            }
            if child_document {
                let decision = response_policy.authorize_frame_ancestors(&self.ancestors);
                self.reports
                    .push_with_policy(&response_policy, decision.violations)?;
                if !decision.allowed {
                    return Err("frame-ancestors blocked standalone response".into());
                }
                crate::frame_embedding::authorize_xfo(
                    url.as_str(),
                    &response.headers,
                    &self.ancestors,
                    &response_policy,
                )?;
            }
            if !self.kind.selects_response_representation()
                && content_type
                    .as_deref()
                    .is_some_and(crate::download::computed_type_is_download)
            {
                return Ok((
                    FrameResourceResponse::PageResourceNoCommit {
                        reason: "binary document requires parent quarantine".into(),
                    },
                    crate::standalone_document_policy::FramePolicy {
                        csp: response_policy,
                        document: self.document_policy.clone(),
                    },
                ));
            }
            let content_language = response
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-language"))
                .map(|(_, value)| value.clone());
            let same_origin = self.document_policy.origin != "null"
                && self.document_policy.origin == url.origin().ascii_serialization();
            let inherited_encoding = if same_origin {
                oryn_net::document_fallback_encoding(Some(&self.parent_encoding)).map(str::to_owned)
            } else {
                None
            };
            let (source, encoding) =
                if oryn_dom::xml::media_document_tag(content_type.as_deref().unwrap_or(""))
                    .is_some()
                {
                    (String::new(), "UTF-8")
                } else {
                    oryn_net::decode_resource_text_with_fallback(
                        &response.body,
                        content_type.as_deref(),
                        crate::network::navigation_text_kind(content_type.as_deref()),
                        inherited_encoding.as_deref(),
                    )
                };
            if source.len() > 16 * 1024 * 1024 {
                return Err("standalone frame decoded source bytes exceeded".into());
            }
            if self.cancel.load(Ordering::Acquire) {
                return Err("standalone frame cancelled during decode".into());
            }
            if url.scheme() != "data" {
                let opaque = self.document_policy.origin == "null";
                let credentials = true;
                let reporting = crate::coep_reporting::Policy::new(
                    self.initiator.as_str(),
                    &self.document_policy.headers,
                );
                self.reports.push_coep(if child_document {
                    reporting.frame(
                        &self.document_policy.origin,
                        opaque,
                        &url,
                        credentials,
                        &response.headers,
                    )
                } else {
                    reporting.response(crate::coep_reporting::ResponsePolicyInputs {
                        origin: &self.document_policy.origin,
                        opaque,
                        target: &url,
                        cors: false,
                        credentials,
                        headers: &response.headers,
                        destination: self.kind.resource_destination(),
                    })
                });
                self.isolation.authorize_response(
                    &self.document_policy.origin,
                    opaque,
                    &url,
                    false,
                    credentials,
                    &response.headers,
                )?;
                if child_document {
                    self.isolation.authorize_frame_response(&response.headers)?;
                }
            }
            let policy_headers = if url.scheme() == "blob" {
                &self
                    .local_blob
                    .as_ref()
                    .ok_or("blob policy absent")?
                    .document
                    .headers
            } else if url.scheme() == "data" {
                &self.document_policy.headers
            } else {
                &response.headers
            };
            let document_policy = self.document_policy.child(
                url.origin().ascii_serialization(),
                policy_headers,
                self.container_allow.clone(),
                container_source.clone(),
            )?;
            let origin = if url.scheme() == "blob" {
                self.local_blob
                    .as_ref()
                    .map(|blob| blob.origin.clone())
                    .ok_or("blob origin absent")?
            } else {
                oryn_js::origin::Origin::from_url(url.as_str())
            };
            let native_origin = oryn_js::worker_wire::AgentOrigin {
                serialized: origin.serialize(),
                opaque: origin.is_opaque(),
                authority: origin.security_token_key(),
            };
            let elapsed = timing_started.elapsed().as_micros().min(u64::MAX as u128) as u64;
            let mut timing = if matches!(url.scheme(), "data" | "blob") {
                crate::network::local_resource_timing_metadata(
                    &self.document_policy.origin,
                    &response.body,
                    content_type.as_deref(),
                    elapsed,
                )
            } else {
                crate::network::resource_timing_metadata(
                    &self.document_policy.origin,
                    self.document_policy.origin == "null",
                    timing_hops,
                    &response,
                    elapsed,
                )
            };
            timing.navigation_phases = Some(navigation_phases.snapshot());
            _timing.set_details(timing.clone());
            return Ok((
                FrameResourceResponse::PageResource {
                    inherited_encoding,
                    response_headers: response.headers.clone(),
                    raw_body_base64: Some(base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &response.body,
                    )),
                    request_state: Some(oryn_common::navigation_request::NavigationRequestState {
                        initiator_url: request_initiator.clone(),
                        opaque_initiator: request_opaque,
                        referrer: referrer_source
                            .as_ref()
                            .and_then(|source| policy.header(source, &url)),
                        referrer_policy: policy.as_str().into(),
                    }),
                    timing: Some(Box::new(timing)),
                    effective_post: post,
                    stream_id: None,
                    native_origin: Some(native_origin),
                    content_language,
                    document_referrer: referrer_source
                        .as_ref()
                        .and_then(|source| policy.header(source, &url))
                        .unwrap_or_default(),
                    referrer_policy: policy.after_response(&response.headers).as_str().into(),
                    url: url.to_string(),
                    source,
                    destination: self.kind.resource_destination().into(),
                    refresh_url: None,
                    content_type,
                    encoding: encoding.into(),
                    modals_allowed: response_policy
                        .sandbox()
                        .is_none_or(|sandbox| sandbox.allow_modals),
                    worker_policy_token: None,
                },
                crate::standalone_document_policy::FramePolicy {
                    csp: response_policy,
                    document: document_policy,
                },
            ));
        }
        Err("standalone frame did not complete".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_resource_response::FrameResourceResponse;
    use crate::network::{NetworkBroker, NetworkError, NetworkRequest, NetworkResponse};
    #[test]
    fn local_frame_capture_without_broker_preserves_http_denial_and_cancel() {
        let mut page = RustExecutableDocument::load("<iframe id=child></iframe>").unwrap();
        let parent = page.top_realm;
        let frame = page.node_by_id("child").unwrap();
        assert!(page.agent.host.network_broker_for_realm(parent).is_none());
        assert!(
            Loader::capture_for_url(&mut page, parent, frame, Some("https://example.test/frame"))
                .is_err()
        );
        let loader = Loader::capture_for_url(
            &mut page,
            parent,
            frame,
            Some("data:text/html,<p>local</p>"),
        )
        .unwrap();
        assert_eq!(
            loader.next, 0,
            "brokerless local ID must not require a retired HTTP endpoint"
        );
        assert!(
            loader.committed_broker().is_none(),
            "local decode must not install a synthetic network endpoint"
        );
        let sibling =
            Loader::capture_for_url(&mut page, parent, frame, Some("data:text/html,sibling"))
                .unwrap();
        assert_eq!(
            sibling.next, 0,
            "independent local response scopes may reuse caller IDs"
        );
        sibling.cancel.store(true, Ordering::Release);
        assert!(
            !loader.cancel.load(Ordering::Acquire),
            "local cancellation belongs to the exact response scope"
        );
        let (response, _) = loader
            .load("data:text/html,<p>local</p>".into(), None, None)
            .unwrap();
        let FrameResourceResponse::PageResource {
            source,
            raw_body_base64,
            native_origin,
            ..
        } = response
        else {
            panic!("local document response")
        };
        assert_eq!(source, "<p>local</p>");
        assert!(raw_body_base64.is_some());
        assert!(native_origin.is_some());
        let loader =
            Loader::capture_for_url(&mut page, parent, frame, Some("data:text/html,canceled"))
                .unwrap();
        loader.cancel.store(true, Ordering::Release);
        assert!(
            loader
                .load("data:text/html,canceled".into(), None, None)
                .err()
                .unwrap()
                .contains("cancelled")
        );
        let loader =
            Loader::capture_for_url(&mut page, parent, frame, Some("data:text/html,local"))
                .unwrap();
        assert!(
            loader
                .load("https://example.test/frame".into(), None, None)
                .is_err(),
            "local admission cannot provide HTTP transport"
        );
    }
    #[test]
    fn embedded_document_commit_starts_without_initial_document_and_reuses_only_its_proxy() {
        for tag in ["object", "embed"] {
            let mut page = RustExecutableDocument::load_with_environment(
                &format!("<body><{tag} id=owner name=ownedFrame></{tag}></body>"),
                &BTreeMap::new(),
                None,
                Some(Url::parse("https://example.test/root").unwrap()),
            )
            .unwrap();
            let parent = page.top_realm;
            let frame = page.node_by_id("owner").unwrap();
            let dom = page.agent.host.automation_dom(parent).unwrap();
            assert_eq!(
                page.agent.host.realm_ids(),
                vec![parent],
                "resource-less embedded element has no initial Document"
            );
            if tag == "object" {
                assert_eq!(
                    page.agent
                        .host
                        .evaluate_string(
                            "String(document.getElementById('owner').contentWindow===null)"
                        )
                        .unwrap(),
                    "true"
                );
            }
            let mut previous = None;
            for value in ["first", "second"] {
                let parsed = oryn_dom::html::parse(&format!("<p id=value>{value}</p>"));
                let document = dom
                    .borrow_mut()
                    .import_subtree(&parsed.dom, parsed.document)
                    .unwrap();
                let realm = page
                    .agent
                    .host
                    .commit_frame_document_with_url(
                        frame,
                        oryn_js::origin::Origin::from_url("https://example.test/child"),
                        dom.clone(),
                        document,
                        Some("https://example.test/child"),
                    )
                    .unwrap();
                assert_eq!(page.agent.host.active_realm(), parent);
                assert!(page.agent.host.network_broker_for_realm(realm).is_none());
                assert_eq!(
                    page.agent
                        .host
                        .evaluate_string("ownedFrame.document.querySelector('#value').textContent")
                        .unwrap(),
                    value
                );
                if let Some(previous) = previous {
                    assert!(!page.agent.host.realm_execution_live(previous));
                    assert_eq!(
                        page.agent
                            .host
                            .evaluate_string("String(savedEmbeddedProxy===ownedFrame)")
                            .unwrap(),
                        "true"
                    );
                } else {
                    assert_eq!(
                        page.agent
                            .host
                            .realm_descriptor(realm)
                            .unwrap()
                            .document_generation,
                        1
                    );
                    page.agent
                        .host
                        .execute("globalThis.savedEmbeddedProxy=ownedFrame")
                        .unwrap();
                }
                previous = Some(realm);
            }
        }
    }

    #[test]
    fn embedded_first_commit_rejects_unowned_network_endpoint_before_creating_realm() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<body><object id=owner></object></body>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/root").unwrap()),
        )
        .unwrap();
        let parent = page.top_realm;
        let frame = page.node_by_id("owner").unwrap();
        let dom = page.agent.host.automation_dom(parent).unwrap();
        let parsed = oryn_dom::html::parse("<p>child</p>");
        let document = dom
            .borrow_mut()
            .import_subtree(&parsed.dom, parsed.document)
            .unwrap();
        let realms = page.agent.host.realm_ids();
        let error = page
            .agent
            .host
            .commit_frame_document_with_resource_owners(crate::host::FrameResourceCommitInput {
                frame,
                origin: oryn_js::origin::Origin::from_url("https://example.test/child"),
                dom,
                document,
                committed_url: Some("https://example.test/child"),
                resource_broker: Some(SharedNetworkBroker::new(crate::network::UnavailableBroker)),
                #[cfg(unix)]
                resource_client: None,
                ancestor_capture: None,
            })
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("frame resource broker unavailable")
        );
        assert_eq!(page.agent.host.realm_ids(), realms);
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(document.getElementById('owner').contentWindow===null)")
                .unwrap(),
            "true"
        );
        assert!(page.agent.host.network_broker_for_realm(parent).is_none());
    }

    struct Fixture;
    impl NetworkBroker for Fixture {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            let (mime, body) = if request.url.ends_with("/frame.html") {
                (
                    "text/html",
                    r#"<!doctype html><link rel=stylesheet href=frame.css><script>globalThis.order=['inline'];document.write('<script src="written.js"><\/script>')</script><script type=module src=entry.js></script><article>child</article>"#,
                )
            } else if request.url.ends_with("/frame.css") {
                ("text/css", "article{color:rgb(12, 34, 56)}")
            } else if request.url.ends_with("/written.js") {
                ("text/javascript", "order.push('written')")
            } else if request.url.ends_with("/entry.js") {
                (
                    "text/javascript",
                    "import {value} from './dependency.js';order.push(value)",
                )
            } else if request.url.ends_with("/dependency.js") {
                ("text/javascript", "export const value='module'")
            } else {
                return Err(NetworkError::Transport("unmatched fixture resource".into()));
            };
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![("content-type".into(), mime.into())],
                body: body.as_bytes().to_vec(),
            })
        }
    }
    #[test]
    fn credentialless_navigation_keeps_credentials_without_reclassifying_as_cors() {
        struct Capture(Arc<std::sync::Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Capture {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request);
                Err(NetworkError::Transport(
                    "stop after request inspection".into(),
                ))
            }
        }
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut page = RustExecutableDocument::load_with_environment(
            "<body><iframe id=owner></iframe>",
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Capture(seen.clone()))),
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        let realm = page.top_realm;
        let node = page.node_by_id("owner").unwrap();
        let mut loader = Loader::capture(&mut page, realm, node).unwrap();
        assert!(
            loader.committed_broker().is_some(),
            "an admitted parent broker survives response ownership transfer"
        );
        loader.isolation.embedder = crate::agent_isolation::EmbedderPolicy::Credentialless;
        assert!(
            loader
                .load("https://cross.test/frame".into(), None, None)
                .is_err()
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].credentials);
        assert_eq!(
            seen[0].security.credentials,
            crate::cookies::CookieCredentials::Include
        );
        assert_eq!(
            seen[0].security.fetch_metadata.mode,
            Some(crate::fetch_metadata::Mode::Navigate)
        );
    }

    #[test]
    fn declared_embedded_type_survives_capture_and_does_not_replace_response_headers() {
        struct Untyped(Arc<std::sync::Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Untyped {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![],
                    body: b"<svg xmlns='http://www.w3.org/2000/svg' width='12' height='14'/>"
                        .to_vec(),
                })
            }
        }
        for tag in ["object", "embed"] {
            let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
            let mut page = RustExecutableDocument::load_with_environment(
                &format!("<body><{tag} id=owner type='image/svg+xml'></{tag}>"),
                &BTreeMap::new(),
                Some(SharedNetworkBroker::new(Untyped(seen.clone()))),
                Some(Url::parse("https://example.test/").unwrap()),
            )
            .unwrap();
            let realm = page.top_realm;
            let node = page.node_by_id("owner").unwrap();
            let loader = Loader::capture(&mut page, realm, node).unwrap();
            page.agent
                .host
                .automation_dom(realm)
                .unwrap()
                .borrow_mut()
                .set_attribute(node, "type", "image/png")
                .unwrap();
            let (response, _) = loader.load("/resource".into(), None, None).unwrap();
            let crate::frame_resource_response::FrameResourceResponse::PageResource {
                content_type,
                ..
            } = response
            else {
                panic!("embedded response")
            };
            assert_eq!(content_type.as_deref(), Some("image/svg+xml"));
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(
                seen[0].security.fetch_metadata.mode,
                Some(crate::fetch_metadata::Mode::Navigate)
            );
            assert_eq!(
                seen[0].security.fetch_metadata.destination.as_deref(),
                Some(tag)
            );
        }
    }

    #[test]
    fn nested_frame_loader_retains_embedding_broker_activity_without_sibling_activity() {
        let mut page=RustExecutableDocument::load_with_environment(
            "<body><iframe srcdoc='<iframe id=inner></iframe>'></iframe><iframe id=sibling></iframe>",
            &BTreeMap::new(),Some(SharedNetworkBroker::new(Fixture)),Some(Url::parse("https://example.test/").unwrap())).unwrap();
        let parent = page
            .frame_realms
            .keys()
            .copied()
            .find(|realm| {
                page.agent.host.automation_dom(*realm).is_some_and(|dom| {
                    let document = page.agent.host.automation_realm_identity(*realm).unwrap().0;

                    dom.borrow()
                        .query_selector(document, "#inner")
                        .unwrap()
                        .is_some()
                })
            })
            .unwrap();
        let sibling = page
            .frame_realms
            .iter()
            .find(|(realm, owner)| **realm != parent && owner.parent_realm == page.top_realm)
            .map(|(realm, _)| *realm)
            .unwrap();
        let dom = page.agent.host.automation_dom(parent).unwrap();
        let document = page.agent.host.automation_realm_identity(parent).unwrap().0;
        let frame = dom
            .borrow()
            .query_selector(document, "#inner")
            .unwrap()
            .unwrap();
        let loader = Loader::capture(&mut page, parent, frame).unwrap();
        let parent_broker = page.agent.host.network_broker_for_realm(parent).unwrap();
        let root_broker = page
            .agent
            .host
            .network_broker_for_realm(page.top_realm)
            .unwrap();
        let sibling_broker = page.agent.host.network_broker_for_realm(sibling).unwrap();
        let mut response = loader
            .broker
            .start_stream(NetworkRequest {
                request_id: 17,
                method: "GET".into(),
                url: "https://example.test/frame.html".into(),
                headers: vec![],
                body: vec![],
                credentials: false,
                resolved_addrs: vec![],
                security: Default::default(),
            })
            .unwrap();
        assert_eq!(loader.broker.owned_operation_activity().0, 1);
        assert_eq!(parent_broker.owned_operation_activity().0, 1);
        assert_eq!(root_broker.owned_operation_activity().0, 1);
        assert_eq!(sibling_broker.owned_operation_activity().0, 0);
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut response.body, &mut bytes).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(parent_broker.owned_operation_activity().0, 0);
        assert_eq!(root_broker.owned_operation_activity().0, 0);
    }

    #[test]
    fn post_origin_uses_frozen_replay_policy_and_method_after_redirect() {
        struct Redirect(Arc<std::sync::Mutex<Vec<NetworkRequest>>>, u16);
        impl NetworkBroker for Redirect {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                let first = request.url.ends_with("start");
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if first { self.1 } else { 200 },
                    headers: if first {
                        vec![
                            ("location".into(), "/end".into()),
                            ("referrer-policy".into(), "no-referrer".into()),
                        ]
                    } else {
                        vec![("content-type".into(), "text/html".into())]
                    },
                    body: b"<body>done".to_vec(),
                })
            }
        }
        for (status, opaque) in [(302, false), (307, false), (307, true)] {
            let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
            let mut page = RustExecutableDocument::load_with_environment(
                "<body><iframe id=owner></iframe>",
                &BTreeMap::new(),
                Some(SharedNetworkBroker::new(Redirect(seen.clone(), status))),
                Some(Url::parse("https://successor.example/").unwrap()),
            )
            .unwrap();
            let realm = page.top_realm;
            let frame = page.node_by_id("owner").unwrap();
            let mut loader = Loader::capture(&mut page, realm, frame).unwrap();
            loader.navigation.history = true;
            loader.request_state = Some(oryn_common::navigation_request::NavigationRequestState {
                initiator_url: Some("https://original.example/private".into()),
                opaque_initiator: opaque,
                referrer: Some("https://original.example/".into()),
                referrer_policy: "unsafe-url".into(),
            });
            loader
                .load(
                    "/start".into(),
                    None,
                    Some(oryn_js::navigation::PostResource {
                        bytes: b"x=1".to_vec(),
                        content_type: "application/x-www-form-urlencoded".into(),
                    }),
                )
                .unwrap();
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 2);
            let origin = |i: usize| {
                seen[i]
                    .headers
                    .iter()
                    .find(|(name, _)| name == "origin")
                    .map(|(_, value)| value.as_str())
            };
            assert_eq!(
                origin(0),
                Some(if opaque {
                    "null"
                } else {
                    "https://original.example"
                })
            );
            assert_eq!(origin(1), if status == 302 { None } else { Some("null") });
            assert_eq!(seen[1].method, if status == 302 { "GET" } else { "POST" });
        }
    }

    #[test]
    fn frame_redirects_preserve_reduced_referrer_and_schemeful_cookie_site() {
        struct Redirects(Arc<std::sync::Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Redirects {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                let location = if request.url.ends_with("start") {
                    Some("https://b.example.test/middle")
                } else if request.url.ends_with("middle") {
                    Some("https://a.example.test/end")
                } else {
                    None
                };
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if location.is_some() { 302 } else { 200 },
                    headers: location
                        .map(|location| {
                            vec![
                                ("location".into(), location.into()),
                                (
                                    "referrer-policy".into(),
                                    if request.url.ends_with("middle") {
                                        "unsafe-url"
                                    } else {
                                        "strict-origin-when-cross-origin"
                                    }
                                    .into(),
                                ),
                            ]
                        })
                        .unwrap_or_else(|| vec![("content-type".into(), "text/html".into())]),
                    body: b"<body>child".to_vec(),
                })
            }
        }
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut page = RustExecutableDocument::load_with_environment(
            "<body><iframe id=owner></iframe>",
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Redirects(seen.clone()))),
            Some(Url::parse("https://a.example.test/private?q=secret").unwrap()),
        )
        .unwrap();
        let realm = page.top_realm;
        let frame = page.node_by_id("owner").unwrap();
        let loader = Loader::capture(&mut page, realm, frame).unwrap();
        let (response, _) = loader.load("/start".into(), None, None).unwrap();
        let crate::frame_resource_response::FrameResourceResponse::PageResource {
            request_state: Some(state),
            ..
        } = response
        else {
            panic!("frame request provenance")
        };
        assert_eq!(
            state.initiator_url.as_deref(),
            Some("https://a.example.test/private?q=secret")
        );
        assert_eq!(state.referrer.as_deref(), Some("https://a.example.test/"));
        assert_eq!(state.referrer_policy, "unsafe-url");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(
            seen[2]
                .headers
                .iter()
                .find(|(name, _)| name == "referer")
                .map(|(_, value)| value.as_str()),
            Some("https://a.example.test/")
        );
        assert!(
            seen.iter()
                .all(|request| !request.security.redirect_chain_cross_site)
        );
    }
    #[test]
    fn standalone_history_replays_original_initiator_and_suppressed_referrer() {
        struct Capture(Arc<std::sync::Mutex<Vec<NetworkRequest>>>);
        impl NetworkBroker for Capture {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.0.lock().unwrap().push(request.clone());
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/html".into())],
                    body: b"<body>restored".to_vec(),
                })
            }
        }
        for opaque in [false, true] {
            for reload in [false, true] {
                let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
                let mut page = RustExecutableDocument::load_with_environment(
                    "<iframe id=owner></iframe>",
                    &BTreeMap::new(),
                    Some(SharedNetworkBroker::new(Capture(seen.clone()))),
                    Some(Url::parse("https://current.test/private").unwrap()),
                )
                .unwrap();
                let realm = page.top_realm;
                let frame = page.node_by_id("owner").unwrap();
                let state = oryn_common::navigation_request::NavigationRequestState {
                    initiator_url: Some("https://original.test/private".into()),
                    opaque_initiator: opaque,
                    referrer: None,
                    referrer_policy: "no-referrer".into(),
                };
                let mut loader = Loader::capture(&mut page, realm, frame).unwrap();
                loader.request_state = Some(state.clone());
                loader.navigation = oryn_js::platform::NavigationRequestFlags {
                    reload,
                    history: !reload,
                };
                let (response, _) = loader
                    .load(
                        "https://current.test/child".into(),
                        Some("unsafe-url".into()),
                        None,
                    )
                    .unwrap();
                let crate::frame_resource_response::FrameResourceResponse::PageResource {
                    request_state,
                    document_referrer,
                    ..
                } = response
                else {
                    panic!("history frame")
                };
                assert_eq!(request_state, Some(state));
                assert!(document_referrer.is_empty());
                {
                    let seen = seen.lock().unwrap();
                    let request = seen.last().unwrap();
                    assert!(
                        !request
                            .headers
                            .iter()
                            .any(|(name, _)| name.eq_ignore_ascii_case("referer"))
                    );
                    assert_eq!(
                        request.security.initiator_url.as_deref(),
                        Some(if opaque {
                            "about:blank"
                        } else {
                            "https://original.test/private"
                        })
                    );
                    let mut request = request.clone();
                    crate::fetch_metadata::apply(&mut request).unwrap();
                    assert!(
                        request
                            .headers
                            .contains(&("sec-fetch-site".into(), "cross-site".into()))
                    );
                }
            }
        }
    }
    #[test]
    fn standalone_attachment_never_decodes_or_replaces_the_child_document() {
        struct Attachment;
        impl NetworkBroker for Attachment {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![
                        (
                            "content-disposition".into(),
                            "attachment; filename=payload.html".into(),
                        ),
                        ("x-frame-options".into(), "DENY".into()),
                    ],
                    body: b"<script>throw Error('download executed')</script>".to_vec(),
                })
            }
        }
        let mut page = RustExecutableDocument::load_with_environment(
            "<iframe id=owner></iframe>",
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Attachment)),
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        let realm = page.top_realm;
        let frame = page.node_by_id("owner").unwrap();
        let loader = Loader::capture(&mut page, realm, frame).unwrap();
        let (response, _) = loader.load("payload".into(), None, None).unwrap();
        assert!(
            matches!(response,crate::frame_resource_response::FrameResourceResponse::PageResourceNoCommit{reason}if reason.contains("quarantine"))
        );
    }
    #[test]
    fn standalone_document_sniff_preserves_plain_and_rejects_mislabeled_binary() {
        struct Fixture;
        impl NetworkBroker for Fixture {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let binary = request.url.ends_with("binary");
                let unknown = request.url.ends_with("unknown");
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: if unknown {
                        vec![]
                    } else {
                        vec![("content-type".into(), "text/plain".into())]
                    },
                    body: if binary {
                        b"PK\x03\x04\x00".to_vec()
                    } else {
                        b"<!--comment--><p id=marker>text</p>".to_vec()
                    },
                })
            }
        }
        let mut page = RustExecutableDocument::load_with_environment(
            "<iframe id=owner></iframe>",
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Fixture)),
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        let realm = page.top_realm;
        let frame = page.node_by_id("owner").unwrap();
        for (target, expected) in [
            ("plain", Some("text/plain")),
            ("unknown", Some("text/html")),
            ("binary", None),
        ] {
            let loader = Loader::capture(&mut page, realm, frame).unwrap();
            let (response, _) = loader.load(target.into(), None, None).unwrap();
            match expected {
                Some(expected) => {
                    let FrameResourceResponse::PageResource {
                        content_type,
                        source,
                        ..
                    } = response
                    else {
                        panic!("expected document")
                    };
                    assert_eq!(content_type.as_deref(), Some(expected));
                    assert!(source.contains("id=marker"));
                }
                None => assert!(matches!(
                    response,
                    FrameResourceResponse::PageResourceNoCommit { .. }
                )),
            }
        }
    }
    #[test]
    fn standalone_frame_encoding_is_captured_and_scoped_to_final_origin() {
        struct EncodingFixture;
        impl NetworkBroker for EncodingFixture {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                let location = if request.url.ends_with("/away") {
                    Some("https://other.test/final")
                } else if request.url.ends_with("/home") {
                    Some("https://example.test/final")
                } else {
                    None
                };
                let mime = if request.url.ends_with("/header") {
                    "text/html;charset=UTF-8"
                } else if request.url.ends_with("/plain") {
                    "text/plain"
                } else {
                    "text/html"
                };
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: if location.is_some() { 302 } else { 200 },
                    headers: location
                        .map(|url| vec![("location".into(), url.into())])
                        .unwrap_or_else(|| {
                            vec![
                                ("content-type".into(), "application/octet-stream".into()),
                                ("Content-Type".into(), mime.into()),
                            ]
                        }),
                    body: vec![0xca],
                })
            }
        }
        for (target, opaque, expected, encoding) in [
            ("https://example.test/final", false, "К", "windows-1251"),
            ("https://other.test/final", false, "Ê", "windows-1252"),
            ("https://example.test/away", false, "Ê", "windows-1252"),
            ("https://other.test/home", false, "К", "windows-1251"),
            ("https://example.test/header", false, "�", "UTF-8"),
            ("https://example.test/plain", false, "К", "windows-1251"),
            ("https://example.test/final", true, "Ê", "windows-1252"),
        ] {
            let mut page = RustExecutableDocument::load_with_environment(
                "<iframe id=owner></iframe>",
                &BTreeMap::new(),
                Some(SharedNetworkBroker::new(EncodingFixture)),
                Some(Url::parse("https://example.test/").unwrap()),
            )
            .unwrap();
            let realm = page.top_realm;
            let frame = page.node_by_id("owner").unwrap();
            let dom = page.agent.host.automation_dom(realm).unwrap();
            let document = dom.borrow().document();
            dom.borrow_mut()
                .arena_mut()
                .get_mut(document)
                .unwrap()
                .document_metadata
                .as_mut()
                .unwrap()
                .encoding = "windows-1251".into();
            let mut loader = Loader::capture(&mut page, realm, frame).unwrap();
            dom.borrow_mut()
                .arena_mut()
                .get_mut(document)
                .unwrap()
                .document_metadata
                .as_mut()
                .unwrap()
                .encoding = "UTF-8".into();
            if opaque {
                loader.document_policy.rebase_origin("null".into());
            }
            let (response, _) = loader.load(target.into(), None, None).unwrap();
            let crate::frame_resource_response::FrameResourceResponse::PageResource {
                source,
                encoding: actual,
                ..
            } = response
            else {
                panic!("expected frame response")
            };
            assert_eq!(source, expected, "{target}, opaque={opaque}");
            assert_eq!(actual, encoding, "{target}, opaque={opaque}");
        }
    }

    #[test]
    fn standalone_frame_cancellation_prevents_broker_admission() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<body><iframe id=owner></iframe>",
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(Fixture)),
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        let realm = page.top_realm;
        let frame = page.node_by_id("owner").unwrap();
        let loader = Loader::capture(&mut page, realm, frame).unwrap();
        loader.cancel.store(true, Ordering::Release);
        assert!(
            loader
                .load("unmatched".into(), None, None)
                .unwrap_err()
                .contains("cancelled")
        );
    }
    #[test]
    fn live_container_policy_mutation_is_visible_without_reloading_child() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<iframe id=owner srcdoc='<p>child</p>'></iframe>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        assert_eq!(page.agent.host.evaluate_string(r#"(()=>{
            const frame=document.getElementById('owner'),child=frame.contentWindow,original=child.document;
            frame.allow="document-domain 'none'";let denied=false;try{child.document.domain='example.test'}catch(e){denied=e.name==='SecurityError'}
            frame.allow='document-domain *';child.document.domain='example.test';
            return String(denied&&child.document===original&&child.document.domain==='example.test');
        })()"#).unwrap(),"true");
    }
    #[test]
    fn standalone_root_cluster_header_is_admitted_before_author_script() {
        for (headers, expected) in [
            (vec![("Origin-Agent-Cluster", "?1")], "true:ignored"),
            (
                vec![("Origin-Agent-Cluster-Report-Only", "?1")],
                "false:SecurityError",
            ),
            (
                vec![
                    ("Origin-Agent-Cluster", "?1"),
                    ("Origin-Agent-Cluster", "?1"),
                ],
                "false:SecurityError",
            ),
        ] {
            let mut resources = BTreeMap::new();
            resources.insert(
                "oryn:response-policy-headers".into(),
                serde_json::to_string(&headers).unwrap(),
            );
            let mut page=RustExecutableDocument::load_with_environment("<script>globalThis.clusterOutcome=String(originAgentCluster)+':';try{document.domain='invalid.other.test';clusterOutcome+='ignored'}catch(e){clusterOutcome+=e.name}</script>",&resources,None,Some(Url::parse("https://example.test/").unwrap())).unwrap();
            assert_eq!(
                page.agent.host.evaluate_string("clusterOutcome").unwrap(),
                expected
            );
        }
    }
    #[test]
    fn response_permissions_policy_is_installed_before_child_authored_script() {
        struct PolicyFixture;
        impl NetworkBroker for PolicyFixture {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                Ok(NetworkResponse{response_policy: None,timing:None,request_id:request.request_id,status:200,headers:vec![("content-type".into(),"text/html".into()),("permissions-policy".into(),"document-domain=()".into())],body:br#"<script>globalThis.policyOutcome='allowed';try{document.domain='example.test'}catch(e){policyOutcome=e.name}</script>"#.to_vec()})
            }
        }
        let mut page = RustExecutableDocument::load_with_environment(
            "<iframe src='/child'></iframe>",
            &BTreeMap::new(),
            Some(SharedNetworkBroker::new(PolicyFixture)),
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            page.poll_context_events().unwrap();
            let outcome = page
                .agent
                .host
                .evaluate_string("frames[0]&&frames[0].policyOutcome||''")
                .unwrap();
            if !outcome.is_empty() {
                assert_eq!(outcome, "SecurityError");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                page.diagnostics
            );
            std::thread::yield_now();
        }
    }
    #[test]
    fn standalone_frame_io_retains_incumbent_and_rejects_late_response_after_replacement_or_stop() {
        struct Held {
            entered: Arc<AtomicBool>,
            release: Arc<AtomicBool>,
            returned: Arc<AtomicBool>,
            calls: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl NetworkBroker for Held {
            fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
                self.calls.fetch_add(1, Ordering::AcqRel);
                self.entered.store(true, Ordering::Release);
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while !self.release.load(Ordering::Acquire) {
                    if std::time::Instant::now() >= deadline {
                        return Err(NetworkError::Transport(
                            "held frame fixture timed out".into(),
                        ));
                    }
                    std::thread::yield_now();
                }
                self.returned.store(true, Ordering::Release);
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/html".into())],
                    body: b"<!doctype html><script>parent.lateScript=true</script><p>late</p>"
                        .to_vec(),
                })
            }
        }
        struct Release(Arc<AtomicBool>);
        impl Drop for Release {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        for stop in [false, true] {
            let entered = Arc::new(AtomicBool::new(false));
            let release = Arc::new(AtomicBool::new(false));
            let returned = Arc::new(AtomicBool::new(false));
            let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let _release = Release(release.clone());
            let broker = SharedNetworkBroker::new(Held {
                entered: entered.clone(),
                release: release.clone(),
                returned: returned.clone(),
                calls: calls.clone(),
            });
            let mut page = RustExecutableDocument::load_with_environment(
                "<!doctype html><iframe id=f srcdoc='<p>incumbent</p>'></iframe>",
                &BTreeMap::new(),
                Some(broker),
                Some(Url::parse("https://example.test/").unwrap()),
            )
            .unwrap();
            page.agent.host.activate_realm(page.top_realm).unwrap();
            page.agent.host.execute("globalThis.lateScript=false;const f=document.getElementById('f');f.removeAttribute('srcdoc');f.src='held.html'").unwrap();
            page.drain_dynamic_frames().unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !entered.load(Ordering::Acquire) {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert_eq!(calls.load(Ordering::Acquire), 1);
            assert!(page.frame_resources_pending(Some(page.top_realm)));
            assert_eq!(
                page.agent
                    .host
                    .evaluate_string("f.contentDocument.querySelector('p').textContent")
                    .unwrap(),
                "incumbent"
            );
            // Neither a new source nor stop may let the already accepted IO
            // completion replace the selected successor or execute old script.
            if stop {
                page.stop_pending_document_load().unwrap();
            } else {
                page.agent
                    .host
                    .execute("f.srcdoc='<p>successor</p>'")
                    .unwrap();
                page.drain_dynamic_frames().unwrap();
            }
            release.store(true, Ordering::Release);
            while !returned.load(Ordering::Acquire) {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            loop {
                page.drain_tasks().unwrap();
                if page.frame_departures.is_empty() && !page.frame_scripts_pending(None) {
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            page.agent.host.activate_realm(page.top_realm).unwrap();
            assert_eq!(
                page.agent
                    .host
                    .evaluate_string(
                        "String(lateScript)+':'+f.contentDocument.querySelector('p').textContent"
                    )
                    .unwrap(),
                if stop {
                    "false:incumbent"
                } else {
                    "false:successor"
                }
            );
            assert_eq!(calls.load(Ordering::Acquire), 1);
            assert!(page.frame_resource_preparations.is_empty());
        }
    }
    #[test]
    fn standalone_snapshot_and_stream_frames_schedule_classic_written_module_and_style_resources() {
        for snapshot in [false, true] {
            let broker = SharedNetworkBroker::new(Fixture);
            let base = Some(Url::parse("https://example.test/").unwrap());
            let source = "<!doctype html><iframe src=frame.html></iframe>";
            let mut page = if snapshot {
                RustExecutableDocument::load_parsed_with_schedule(
                    crate::html::parse(source),
                    &BTreeMap::new(),
                    Some(broker),
                    base,
                    256 * 1024 * 1024,
                    true,
                    None,
                )
            } else {
                RustExecutableDocument::load_stream_with_limits(
                    DocumentStream::html(Box::new(std::iter::once(Ok(source.into())))),
                    &BTreeMap::new(),
                    Some(broker),
                    base,
                    256 * 1024 * 1024,
                    true,
                )
            }
            .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                page.poll_context_events().unwrap();
                if page
                    .agent
                    .host
                    .evaluate_string(
                        "!!frames[0]&&!!frames[0].order&&frames[0].order.includes('module')",
                    )
                    .unwrap()
                    == "true"
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "{:?}",
                    page.diagnostics
                );
                std::thread::yield_now();
            }
            assert_eq!(
                page.agent
                    .host
                    .evaluate_string("frames[0].order.join(',')")
                    .unwrap(),
                "inline,written,module"
            );
            assert_eq!(page.agent.host.evaluate_string("frames[0].getComputedStyle(frames[0].document.querySelector('article')).color").unwrap(),"rgb(12, 34, 56)");
        }
    }
}
