//! Parent-authenticated policy provenance travels with an accepted form load.
//! No author-provided origin or destination can replace this immutable snapshot.

/// Inputs for issue script navigation authority.
pub(crate) struct ScriptNavigationInputs<'a> {
    pub(crate) source: PageId,
    pub(crate) source_creator: Creator,
    pub(crate) destination: PageId,
    pub(crate) target_creator: Creator,
    pub(crate) destination_url: &'a str,
    pub(crate) source_url: &'a str,
    pub(crate) referrer_policy: &'a str,
}
use super::*;
use oryn_js::{navigation::FormNavigationAuthority, worker_wire::Creator};
use sha2::{Digest, Sha256};
const MAX_AUTHORITY_BYTES: usize = 262_144;
#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot {
    version: u8,
    context: u64,
    source: PageId,
    destination: PageId,
    creator: Creator,
    origin: String,
    fetch: oryn_js::navigation::FormNavigationSource,
    policy: crate::csp::CspPolicySet,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Sealed {
    payload: Vec<u8>,
    mac: [u8; 32],
}
#[derive(serde::Serialize, serde::Deserialize)]
struct ScriptSnapshot {
    version: u8,
    context: u64,
    source: PageId,
    destination: PageId,
    source_creator: Creator,
    target_creator: Creator,
    destination_url: String,
    #[serde(default)]
    service_source: Option<oryn_js::navigation::ServiceNavigationAuthority>,
    #[serde(default)]
    javascript_policy: Option<crate::csp::CspPolicySet>,
    #[serde(default)]
    javascript_origin: Option<String>,
    fetch: oryn_js::navigation::FormNavigationSource,
}
fn script_mac(key: &[u8; 32], payload: &[u8]) -> [u8; 32] {
    // Domain separation prevents casting either token into the other kind.
    let mut bytes = b"oryn.script-navigation.v1\0".to_vec();
    bytes.extend_from_slice(payload);
    mac(key, &bytes)
}
fn mac(key: &[u8; 32], bytes: &[u8]) -> [u8; 32] {
    // RFC 2104 HMAC-SHA256. The fixed-size key is shorter than SHA256's block.
    let mut inner = [0x36u8; 64];
    let mut outer = [0x5cu8; 64];
    for (i, byte) in key.iter().enumerate() {
        inner[i] ^= byte;
        outer[i] ^= byte;
    }
    let mut hash = Sha256::new();
    hash.update(inner);
    hash.update(bytes);
    let digest = hash.finalize();
    let mut hash = Sha256::new();
    hash.update(outer);
    hash.update(digest);
    hash.finalize().into()
}
impl BrowserContext {
    #[cfg(unix)]
    pub(crate) fn navigation_network_redirect_limit(&self) -> usize {
        self.inner.config.max_redirects
    }

    pub(crate) fn issue_script_navigation_authority(
        &self,
        params: ScriptNavigationInputs<'_>,
    ) -> Result<oryn_js::navigation::ScriptNavigationAuthority, RuntimeError> {
        let ScriptNavigationInputs {
            source,
            source_creator,
            destination,
            target_creator,
            destination_url,
            source_url,
            referrer_policy,
        } = params;

        if destination_url.len() > 65_536 {
            return Err(RuntimeError::ProtocolViolation(
                "script navigation URL exceeds bound".into(),
            ));
        }
        let destination_url = url::Url::parse(destination_url)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?
            .to_string();
        if !self.native_target_navigation_allowed(
            source,
            &source_creator,
            destination,
            &target_creator,
        )? {
            return Err(RuntimeError::ContextService(
                "native script target navigation denied".into(),
            ));
        }
        // Reuse the native metadata snapshot, without applying form-action:
        // the independently signed script token never authorizes a form load.
        let metadata = self.issue_form_authority_with_metadata(
            source,
            source_creator.clone(),
            Some(referrer_policy),
            Some(source_url),
        )?;
        let metadata = self.open_form_authority(&metadata, source)?;
        let javascript = destination_url.starts_with("javascript:");
        let snapshot = ScriptSnapshot {
            version: 1,
            context: self.inner.id,
            source,
            destination,
            source_creator,
            target_creator,
            destination_url,
            javascript_policy: javascript.then_some(metadata.policy),
            javascript_origin: javascript.then_some(metadata.origin),
            fetch: metadata.fetch,
            service_source: None,
        };
        self.seal_script_navigation_authority(&snapshot)
    }

    pub(crate) fn issue_document_script_navigation_authority(
        &self,
        destination: PageId,
        source: &oryn_dom::DocumentProvenance,
        target_creator: Creator,
        destination_url: &str,
        referrer_policy: Option<&str>,
    ) -> Result<oryn_js::navigation::ScriptNavigationAuthority, RuntimeError> {
        if destination_url.len() > 65_536 {
            return Err(RuntimeError::ProtocolViolation(
                "script navigation URL exceeds bound".into(),
            ));
        }
        let destination_url = url::Url::parse(destination_url)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?
            .to_string();
        let document = self.open_document_source(destination, source)?;
        if !self.document_target_navigation_allowed(destination, source, &target_creator)? {
            return Err(RuntimeError::ContextService(
                "document script target navigation denied".into(),
            ));
        }
        let user_activation = !document.inert
            && self
                .navigation_source_activation(document.source.source, &document.source.creator)?;
        let mut fetch = document.source.fetch;
        fetch.user_activation = user_activation;
        if let Some(policy) =
            referrer_policy.and_then(crate::referrer::ReferrerPolicy::parse_attribute)
        {
            fetch.referrer_policy = policy.as_str().into();
        }
        self.seal_script_navigation_authority(&ScriptSnapshot {
            version: 1,
            context: self.inner.id,
            source: document.source.source,
            destination,
            source_creator: document.source.creator,
            target_creator,
            javascript_policy: destination_url
                .starts_with("javascript:")
                .then_some(document.source.policy),
            javascript_origin: destination_url
                .starts_with("javascript:")
                .then_some(document.source.origin),
            destination_url,
            fetch,
            service_source: None,
        })
    }

    fn seal_script_navigation_authority(
        &self,
        snapshot: &ScriptSnapshot,
    ) -> Result<oryn_js::navigation::ScriptNavigationAuthority, RuntimeError> {
        let payload = serde_json::to_vec(&snapshot)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        if payload.len() > MAX_AUTHORITY_BYTES / 4 {
            return Err(RuntimeError::ProtocolViolation(
                "script navigation source exceeds bound".into(),
            ));
        }
        let bytes = serde_json::to_vec(&Sealed {
            mac: script_mac(self.form_key()?, &payload),
            payload,
        })
        .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        if bytes.len() > MAX_AUTHORITY_BYTES {
            return Err(RuntimeError::ProtocolViolation(
                "sealed script navigation source exceeds bound".into(),
            ));
        }
        Ok(oryn_js::navigation::ScriptNavigationAuthority(bytes))
    }
    pub(crate) fn script_navigation_source(
        &self,
        authority: &oryn_js::navigation::ScriptNavigationAuthority,
        destination: PageId,
        target_creator: &Creator,
        destination_url: &str,
    ) -> Result<oryn_js::navigation::FormNavigationSource, RuntimeError> {
        Ok(self
            .open_script_navigation_source(authority, destination, target_creator, destination_url)?
            .fetch)
    }
    #[cfg(unix)]
    pub(crate) fn script_navigation_network_source(
        &self,
        authority: &oryn_js::navigation::ScriptNavigationAuthority,
        destination: PageId,
        target_creator: &Creator,
        destination_url: &str,
    ) -> Result<(oryn_js::navigation::FormNavigationSource, bool), RuntimeError> {
        let snapshot = self.open_script_navigation_source(
            authority,
            destination,
            target_creator,
            destination_url,
        )?;
        self.check_javascript_navigation(&snapshot, destination, target_creator, true)?;
        Ok((snapshot.fetch, snapshot.service_source.is_some()))
    }
    fn check_javascript_navigation(
        &self,
        snapshot: &ScriptSnapshot,
        destination: PageId,
        target_creator: &Creator,
        report: bool,
    ) -> Result<(), RuntimeError> {
        if snapshot.destination_url.starts_with("javascript:") {
            let Creator::Window {
                target,
                document_generation,
                stream_epoch,
            } = target_creator
            else {
                return Err(RuntimeError::NavigationCancelled);
            };
            let (owner, _, _) = self.window_worker_creator(
                destination,
                target,
                *document_generation,
                *stream_epoch,
            )?;
            let target_policy = self.window_worker_policy(
                destination,
                target,
                *document_generation,
                *stream_epoch,
            )?;
            let target_domain = self
                .native_document_domain_identity(&owner)
                .map_err(RuntimeError::ContextService)?;
            if snapshot.fetch.source_domain_key.as_deref()
                != Some(target_domain.security_token_key().as_str())
                || crate::csp::CspPolicySet::from_agent_policy(&target_policy)
                    .sandbox()
                    .is_some_and(|flags| !flags.allow_scripts)
            {
                return Err(RuntimeError::NavigationCancelled);
            }
            let policy = snapshot
                .javascript_policy
                .as_ref()
                .ok_or(RuntimeError::NavigationCancelled)?;
            let decision = policy.authorize(&crate::csp::CspRequest {
                destination: crate::csp::CspDestination::JavascriptNavigation,
                url: snapshot.destination_url.clone(),
                inline_source: Some(snapshot.destination_url.clone()),
                nonce: None,
                parser_inserted: false,
                initiated_by_trusted_script: false,
            });
            if report {
                self.handle_csp_violations_for_creator(
                    snapshot.source,
                    decision.violations,
                    Some(policy),
                    Some(snapshot.source_creator.clone()),
                )?;
            }
            if !decision.allowed {
                return Err(RuntimeError::NavigationCancelled);
            }
        }
        Ok(())
    }
    pub(crate) fn javascript_navigation_origin(
        &self,
        page: PageId,
        admission: &oryn_js::navigation::ScriptNavigationAdmission,
        url: &str,
    ) -> Result<oryn_js::worker_wire::AgentOrigin, RuntimeError> {
        // The response grant is a fresh privileged effect. Revalidate the
        // captured script policy and target domain before committing its bytes.
        let snapshot =
            self.open_script_navigation_source(&admission.authority, page, &admission.target, url)?;
        self.check_javascript_navigation(&snapshot, page, &admission.target, false)?;
        let serialized = snapshot
            .javascript_origin
            .ok_or(RuntimeError::NavigationCancelled)?;
        Ok(oryn_js::worker_wire::AgentOrigin {
            serialized,
            opaque: snapshot.fetch.opaque_origin,
            authority: snapshot.fetch.origin_key,
        })
    }

    fn open_script_navigation_source(
        &self,
        authority: &oryn_js::navigation::ScriptNavigationAuthority,
        destination: PageId,
        target_creator: &Creator,
        destination_url: &str,
    ) -> Result<ScriptSnapshot, RuntimeError> {
        let invalid =
            || RuntimeError::ProtocolViolation("invalid native script navigation authority".into());
        if authority.0.len() > MAX_AUTHORITY_BYTES {
            return Err(invalid());
        }
        let sealed: Sealed = serde_json::from_slice(&authority.0).map_err(|_| invalid())?;
        let expected = script_mac(self.form_key()?, &sealed.payload);
        let mut difference = 0u8;
        for (a, b) in expected.iter().zip(sealed.mac) {
            difference |= *a ^ b;
        }
        if difference != 0 {
            return Err(invalid());
        }
        let snapshot: ScriptSnapshot =
            serde_json::from_slice(&sealed.payload).map_err(|_| invalid())?;
        let url = url::Url::parse(destination_url)
            .map_err(|_| invalid())?
            .to_string();
        if snapshot.version != 1
            || snapshot.context != self.inner.id
            || snapshot.destination != destination
            || &snapshot.target_creator != target_creator
            || snapshot.destination_url != url
        {
            return Err(invalid());
        }
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = target_creator
        else {
            return Err(invalid());
        };
        self.window_worker_creator(destination, target, *document_generation, *stream_epoch)?;
        // The source may retire after acceptance; its sealed metadata remains
        // immutable. The target must still be the admitted document here.
        Ok(snapshot)
    }
    fn form_key(&self) -> Result<&[u8; 32], RuntimeError> {
        self.inner
            .form_navigation_key
            .get_or_init(|| {
                let mut key = [0; 32];
                getrandom::fill(&mut key).map_err(|error| error.to_string())?;
                Ok(key)
            })
            .as_ref()
            .map_err(|error| RuntimeError::ContextService(error.clone()))
    }
    fn seal_form_authority(
        &self,
        snapshot: &Snapshot,
    ) -> Result<FormNavigationAuthority, RuntimeError> {
        let payload = serde_json::to_vec(snapshot)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        if payload.len() > MAX_AUTHORITY_BYTES / 4 {
            return Err(RuntimeError::ProtocolViolation(
                "form policy snapshot exceeds bound".into(),
            ));
        }
        let bytes = serde_json::to_vec(&Sealed {
            mac: mac(self.form_key()?, &payload),
            payload,
        })
        .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        if bytes.len() > MAX_AUTHORITY_BYTES {
            return Err(RuntimeError::ProtocolViolation(
                "sealed form policy exceeds bound".into(),
            ));
        }
        Ok(FormNavigationAuthority(bytes))
    }
    fn open_form_authority(
        &self,
        authority: &FormNavigationAuthority,
        page: PageId,
    ) -> Result<Snapshot, RuntimeError> {
        let invalid =
            || RuntimeError::ProtocolViolation("invalid native form policy authority".into());
        if authority.0.len() > MAX_AUTHORITY_BYTES {
            return Err(invalid());
        }
        let sealed: Sealed = serde_json::from_slice(&authority.0).map_err(|_| invalid())?;
        let expected = mac(self.form_key()?, &sealed.payload);
        let mut difference = 0u8;
        for (a, b) in expected.iter().zip(sealed.mac) {
            difference |= *a ^ b;
        }
        if difference != 0 {
            return Err(invalid());
        }
        let snapshot: Snapshot = serde_json::from_slice(&sealed.payload).map_err(|_| invalid())?;
        if snapshot.version != 1
            || snapshot.context != self.inner.id
            || snapshot.destination != page
        {
            return Err(invalid());
        }
        Ok(snapshot)
    }
    #[cfg(test)]
    pub(crate) fn issue_form_authority(
        &self,
        page: PageId,
        creator: Creator,
    ) -> Result<FormNavigationAuthority, RuntimeError> {
        self.issue_form_authority_with_referrer(page, creator, None)
    }
    #[cfg(test)]
    pub(crate) fn issue_form_authority_with_referrer(
        &self,
        page: PageId,
        creator: Creator,
        referrer: Option<&str>,
    ) -> Result<FormNavigationAuthority, RuntimeError> {
        self.issue_form_authority_with_metadata(page, creator, referrer, None)
    }
    pub(crate) fn issue_form_authority_with_metadata(
        &self,
        page: PageId,
        creator: Creator,
        referrer: Option<&str>,
        current_url: Option<&str>,
    ) -> Result<FormNavigationAuthority, RuntimeError> {
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = &creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "form source must be a native Window".into(),
            ));
        };
        let (owner, _, origin) =
            self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        let policy =
            self.window_worker_policy(page, target, *document_generation, *stream_epoch)?;
        let source_url = if let Some(current_url) = current_url {
            if current_url.len() > 65_536 {
                return Err(RuntimeError::ProtocolViolation(
                    "form source URL exceeds bound".into(),
                ));
            }
            let mut current = url::Url::parse(current_url)
                .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
            let mut committed = url::Url::parse(&policy.url)
                .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
            current.set_fragment(None);
            committed.set_fragment(None);
            let inherited =
                current.scheme() == "about" && matches!(current.path(), "blank" | "srcdoc");
            if !inherited
                && current != committed
                && (origin.serialized == "null"
                    || current.origin().ascii_serialization() != origin.serialized)
            {
                return Err(RuntimeError::ProtocolViolation(
                    "form source URL differs from native document origin".into(),
                ));
            }
            if inherited {
                policy.url.clone()
            } else {
                current_url.to_owned()
            }
        } else {
            policy.url.clone()
        };
        // Hidden candidates are authenticated above but intentionally absent
        // from the public page list used by page_referrer_policy. Their frozen
        // response policy, including native meta updates, belongs to the exact
        // candidate rather than the incumbent's page-level policy.
        let private = self.private_prerender_group(&owner)?.is_some();
        let default_referrer =
            if matches!(target, oryn_js::message_wire::WindowTarget::Root) && !private {
                self.page_referrer_policy(page)?
            } else {
                crate::referrer::ReferrerPolicy::default().after_response(&policy.headers)
            };
        let mut ancestor_urls = self.window_ancestor_urls(&owner)?;
        let top_level_url = ancestor_urls.last().cloned().unwrap_or_else(|| {
            if origin.serialized == "null" {
                "about:blank".into()
            } else {
                policy.url.clone()
            }
        });
        ancestor_urls.insert(
            0,
            if origin.serialized == "null" {
                "about:blank".into()
            } else {
                policy.url.clone()
            },
        );
        let fetch = oryn_js::navigation::FormNavigationSource {
            user_activation: self
                .transient_native_activation(&owner, false)
                .map_err(|error| RuntimeError::ContextService(error.to_string()))?,
            source_domain_key: Some(
                self.native_document_domain_identity(&owner)
                    .map_err(RuntimeError::ContextService)?
                    .security_token_key(),
            ),
            origin_key: origin.authority.clone(),
            url: source_url,
            referrer_policy: referrer
                .and_then(crate::referrer::ReferrerPolicy::parse_attribute)
                .unwrap_or(default_referrer)
                .as_str()
                .into(),
            top_level_url,
            ancestor_urls,
            opaque_origin: origin.serialized == "null",
        };
        let policy = crate::csp::CspPolicySet::from_agent_policy(&policy);
        self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        self.seal_form_authority(&Snapshot {
            version: 1,
            context: self.inner.id,
            source: page,
            destination: page,
            creator,
            origin: origin.serialized,
            fetch,
            policy,
        })
    }
    pub(crate) fn retarget_form_authority(
        &self,
        authority: &FormNavigationAuthority,
        source: PageId,
        destination: PageId,
    ) -> Result<FormNavigationAuthority, RuntimeError> {
        let mut snapshot = self.open_form_authority(authority, source)?;
        snapshot.destination = destination;
        self.seal_form_authority(&snapshot)
    }
    pub(crate) fn form_fetch_source(
        &self,
        authority: &FormNavigationAuthority,
        page: PageId,
    ) -> Result<oryn_js::navigation::FormNavigationSource, RuntimeError> {
        Ok(self.open_form_authority(authority, page)?.fetch)
    }
    pub(crate) fn form_policy_for_navigation(
        &self,
        authority: &FormNavigationAuthority,
        page: PageId,
        url: &str,
    ) -> Result<(String, crate::csp::CspDecision), RuntimeError> {
        self.form_policy_for_navigation_report(authority, page, url, false, None)
    }
    pub(crate) fn form_policy_for_navigation_report(
        &self,
        authority: &FormNavigationAuthority,
        page: PageId,
        url: &str,
        report_allowed: bool,
        original_url: Option<&str>,
    ) -> Result<(String, crate::csp::CspDecision), RuntimeError> {
        let snapshot = self.open_form_authority(authority, page)?;
        if snapshot
            .policy
            .sandbox()
            .is_some_and(|sandbox| !sandbox.allow_forms)
        {
            return Err(RuntimeError::ContextService(
                "source sandbox denies form navigation".into(),
            ));
        }
        let mut decision = snapshot.policy.authorize(&crate::csp::CspRequest {
            destination: crate::csp::CspDestination::Form,
            url: url.into(),
            nonce: None,
            inline_source: None,
            parser_inserted: false,
            initiated_by_trusted_script: false,
        });
        if report_allowed || !decision.allowed {
            let original = original_url.unwrap_or(url);
            for violation in &mut decision.violations {
                // A redirected form violation reports the original navigation
                // URL, with credentials/fragment removed and cross-origin URLs
                // reduced to their origin. Preserve same-origin query text.
                violation.blocked_uri =
                    crate::csp::strip_url_for_report(original, &snapshot.origin);
            }
            #[cfg(unix)]
            self.publish_context_event_batch(
                decision
                    .violations
                    .iter()
                    .map(|violation| ContextEvent::CspViolation {
                        target_page_id: snapshot.source,
                        target_creator: Some(snapshot.creator.clone()),
                        violation: violation.clone(),
                    })
                    .collect(),
            )?;
            for violation in &decision.violations {
                for target in snapshot.policy.report_targets(violation) {
                    self.queue_native_csp_report(
                        Some(snapshot.source),
                        snapshot.fetch.top_level_url.clone(),
                        target,
                        violation.clone(),
                    )?;
                }
            }
        }
        Ok((snapshot.origin, decision))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hmac_uses_standard_sha256_construction() {
        let mut key = [0; 32];
        key[..20].fill(0x0b);
        assert_eq!(
            mac(&key, b"Hi There"),
            [
                0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
                0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
                0x2e, 0x32, 0xcf, 0xf7
            ]
        );
    }
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn form_reports_use_frozen_source_and_original_redirect_url_once() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(
                page,
                "https://source.test/form".into(),
                &[
                    (
                        "content-security-policy".into(),
                        "form-action 'self'".into(),
                    ),
                    (
                        "content-security-policy-report-only".into(),
                        "form-action 'none'".into(),
                    ),
                ],
            )
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let authority = context.issue_form_authority(page, creator.clone()).unwrap();
        let original = "https://source.test/redirect?location=https://other.test/final#private";
        assert!(
            context
                .form_policy_for_navigation(&authority, page, original)
                .unwrap()
                .1
                .allowed
        );
        assert!(
            !context
                .inner
                .context_events
                .lock()
                .unwrap()
                .iter()
                .any(|record| matches!(&record.event, ContextEvent::CspViolation { .. }))
        );
        assert!(
            context
                .form_policy_for_navigation_report(&authority, page, original, true, None)
                .unwrap()
                .1
                .allowed
        );
        let events = context.poll_context_events(page, &creator, 0, 100).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|record| matches!(&record.event, ContextEvent::CspViolation { .. }))
                .count(),
            1
        );
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        context
            .install_document_security(page, "https://replacement.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let decision = context
            .form_policy_for_navigation_report(
                &authority,
                page,
                "https://other.test/final",
                true,
                Some(original),
            )
            .unwrap()
            .1;
        assert!(!decision.allowed);
        assert!(
            decision
                .violations
                .iter()
                .all(|violation| violation.blocked_uri
                    == "https://source.test/redirect?location=https://other.test/final")
        );
        let events = context.inner.context_events.lock().unwrap();
        assert!(
            events
                .iter()
                .filter_map(|record| match &record.event {
                    ContextEvent::CspViolation { target_creator, .. } => Some(target_creator),
                    _ => None,
                })
                .all(|target| target.as_ref() == Some(&creator))
        );
        drop(events);
        let replacement = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 2,
            stream_epoch: 0,
        };
        assert!(
            !context
                .poll_context_events(page, &replacement, 0, 100)
                .unwrap()
                .iter()
                .any(|record| matches!(&record.event, ContextEvent::CspViolation { .. }))
        );
    }
    #[test]
    fn script_source_authority_binds_target_url_and_preserves_retired_source() {
        let context = Browser::new().new_context();
        let id = context.new_page().page_id();
        context
            .install_document_security(id, "https://top.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let root = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let frame = "9".repeat(64);
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://child.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(
                id,
                "https://child.test/",
                &[(
                    "content-security-policy".into(),
                    "form-action 'none'".into(),
                )],
                "",
            )
            .unwrap();
        context
            .bind_frame_worker_policy(id, frame.clone(), 2, 0, "https://child.test/", &ticket)
            .unwrap();
        context
            .register_activation_parent(crate::runtime::worker_service::ActivationParentInputs {
                page: id,
                frame: frame.clone(),
                generation: 2,
                epoch: 0,
                parent: WindowTarget::Root,
                parent_generation: 1,
                parent_epoch: 0,
            })
            .unwrap();
        context
            .register_isolation_container(
                crate::runtime::worker_service::IsolationContainerInputs {
                    page: id,
                    frame: frame.clone(),
                    generation: 2,
                    epoch: 0,
                    allow: String::new(),
                    source: "https://child.test/".into(),
                    sandbox: None,
                },
            )
            .unwrap();
        let source = Creator::Window {
            target: WindowTarget::Frame(frame.clone()),
            document_generation: 2,
            stream_epoch: 0,
        };
        let url = "https://destination.test/next";
        let issue = || {
            context.issue_script_navigation_authority(ScriptNavigationInputs {
                source: id,
                source_creator: source.clone(),
                destination: id,
                target_creator: root.clone(),
                destination_url: url,
                source_url: "https://child.test/history#fragment",
                referrer_policy: "origin",
            })
        };
        assert!(
            issue().is_err(),
            "top navigation still requires source activation"
        );
        let owner = context
            .window_worker_creator(id, &WindowTarget::Frame(frame.clone()), 2, 0)
            .unwrap()
            .0;
        context.record_native_activation(owner.clone()).unwrap();
        let authority = issue().unwrap();
        let fetch = context
            .script_navigation_source(&authority, id, &root, url)
            .unwrap();
        assert!(
            fetch.user_activation,
            "accepted navigation freezes native activation"
        );
        assert_eq!(fetch.url, "https://child.test/history#fragment");
        assert_eq!(
            fetch.origin_key,
            context
                .window_worker_creator(id, &WindowTarget::Frame(frame.clone()), 2, 0)
                .unwrap()
                .2
                .authority
        );
        assert_ne!(
            fetch.origin_key,
            context
                .window_worker_creator(id, &WindowTarget::Root, 1, 0)
                .unwrap()
                .2
                .authority,
            "remote admission retains the source identity rather than borrowing the destination"
        );
        assert_eq!(fetch.referrer_policy, "origin");
        assert_eq!(fetch.top_level_url, "https://top.test/");
        assert_eq!(
            fetch.ancestor_urls,
            vec!["https://child.test/", "https://top.test/"]
        );
        assert!(
            context
                .script_navigation_source(&authority, id, &root, "https://destination.test/other")
                .is_err()
        );
        assert!(
            context
                .script_navigation_source(&authority, id, &source, url)
                .is_err()
        );
        let form = context.issue_form_authority(id, source.clone()).unwrap();
        assert!(
            !context
                .form_policy_for_navigation(&form, id, url)
                .unwrap()
                .1
                .allowed
        );
        assert!(
            context
                .script_navigation_source(
                    &oryn_js::navigation::ScriptNavigationAuthority(form.0),
                    id,
                    &root,
                    url
                )
                .is_err()
        );
        assert!(
            context
                .form_fetch_source(&FormNavigationAuthority(authority.0.clone()), id)
                .is_err()
        );
        assert!(context.transient_native_activation(&owner, true).unwrap());
        assert!(
            context
                .script_navigation_source(&authority, id, &root, url)
                .unwrap()
                .user_activation,
            "consumption after acceptance cannot rewrite signed navigation metadata"
        );
        let mut corrupt = authority.clone();
        let index = corrupt.0.len() / 2;
        corrupt.0[index] ^= 1;
        assert!(
            context
                .script_navigation_source(&corrupt, id, &root, url)
                .is_err()
        );
        assert!(
            Browser::new()
                .new_context()
                .script_navigation_source(&authority, id, &root, url)
                .is_err()
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireWindow {
                    frame,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert_eq!(
            context
                .script_navigation_source(&authority, id, &root, url)
                .unwrap(),
            fetch
        );
        assert!(
            issue().is_err(),
            "a retired source cannot issue a new authority"
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .script_navigation_source(&authority, id, &root, url)
                .is_err()
        );
    }
    #[test]
    fn frame_source_policy_is_frozen_authenticated_and_rebound_only_by_parent() {
        let context = Browser::new().new_context();
        let page = context.new_page();
        let id = page.page_id();
        context
            .install_document_security(
                id,
                "https://top.test/".into(),
                &[(
                    "content-security-policy".into(),
                    "form-action https://top.test".into(),
                )],
            )
            .unwrap();
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let frame = "c".repeat(64);
        let target = WindowTarget::Frame(frame.clone());
        context
            .handle_message_wire(
                id,
                MessageRequest::RegisterWindow {
                    frame: frame.clone(),
                    document_generation: 2,
                    stream_epoch: 0,
                    origin: "https://child.test".into(),
                    inherit_origin: None,
                },
            )
            .unwrap();
        let ticket = context
            .prepare_frame_worker_policy(
                id,
                "https://child.test/",
                &[(
                    "content-security-policy".into(),
                    "form-action https://submit.test".into(),
                )],
                "",
            )
            .unwrap();
        context
            .bind_frame_worker_policy(id, frame.clone(), 2, 0, "https://child.test/", &ticket)
            .unwrap();
        context
            .register_activation_parent(crate::runtime::worker_service::ActivationParentInputs {
                page: id,
                frame: frame.clone(),
                generation: 2,
                epoch: 0,
                parent: WindowTarget::Root,
                parent_generation: 1,
                parent_epoch: 0,
            })
            .unwrap();
        context
            .register_isolation_container(
                crate::runtime::worker_service::IsolationContainerInputs {
                    page: id,
                    frame: frame.clone(),
                    generation: 2,
                    epoch: 0,
                    allow: String::new(),
                    source: "https://child.test/".into(),
                    sandbox: None,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target,
            document_generation: 2,
            stream_epoch: 0,
        };
        let authority = context.issue_form_authority(id, creator.clone()).unwrap();
        let fetch = context.form_fetch_source(&authority, id).unwrap();
        assert_eq!(fetch.url, "https://child.test/");
        assert_eq!(fetch.top_level_url, "https://top.test/");
        assert_eq!(
            fetch.ancestor_urls,
            vec!["https://child.test/", "https://top.test/"]
        );
        assert!(!fetch.opaque_origin);
        let inherited = context
            .issue_form_authority_with_metadata(id, creator.clone(), None, Some("about:srcdoc"))
            .unwrap();
        assert_eq!(
            context.form_fetch_source(&inherited, id).unwrap().url,
            "https://child.test/"
        );
        let rewritten = context
            .issue_form_authority_with_metadata(
                id,
                creator.clone(),
                Some("unsafe-url"),
                Some("https://child.test/history/path#fragment"),
            )
            .unwrap();
        assert_eq!(
            context.form_fetch_source(&rewritten, id).unwrap().url,
            "https://child.test/history/path#fragment"
        );
        assert!(
            context
                .issue_form_authority_with_metadata(
                    id,
                    creator.clone(),
                    None,
                    Some("https://forged.test/")
                )
                .is_err()
        );
        let effective = context
            .issue_form_authority_with_referrer(id, creator.clone(), Some("origin"))
            .unwrap();
        assert_eq!(
            context
                .form_fetch_source(&effective, id)
                .unwrap()
                .referrer_policy,
            "origin"
        );
        let (origin, allowed) = context
            .form_policy_for_navigation(&authority, id, "https://submit.test/receipt")
            .unwrap();
        assert_eq!(origin, "https://child.test");
        assert!(allowed.allowed);
        assert!(
            !context
                .form_policy_for_navigation(&authority, id, "https://top.test/receipt")
                .unwrap()
                .1
                .allowed
        );
        let destination = context.new_page();
        assert!(
            context
                .form_policy_for_navigation(
                    &authority,
                    destination.page_id(),
                    "https://submit.test/"
                )
                .is_err()
        );
        let moved = context
            .retarget_form_authority(&authority, id, destination.page_id())
            .unwrap();
        assert!(
            context
                .form_policy_for_navigation(&moved, destination.page_id(), "https://submit.test/")
                .unwrap()
                .1
                .allowed
        );
        assert_eq!(
            context
                .form_fetch_source(&moved, destination.page_id())
                .unwrap(),
            fetch
        );
        let other = Browser::new().new_context();
        assert!(
            other
                .form_policy_for_navigation(&authority, id, "https://submit.test/")
                .is_err()
        );
        let mut corrupt = authority.clone();
        let middle = corrupt.0.len() / 2;
        corrupt.0[middle] ^= 1;
        assert!(
            context
                .form_policy_for_navigation(&corrupt, id, "https://submit.test/")
                .is_err()
        );
        context
            .handle_message_wire(
                id,
                MessageRequest::RetireWindow {
                    frame,
                    document_generation: 2,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(context.issue_form_authority(id, creator).is_err());
        assert_eq!(
            context
                .form_fetch_source(&moved, destination.page_id())
                .unwrap(),
            fetch,
            "destination and source retirement cannot replace accepted initiator metadata"
        );
        assert!(
            context
                .form_policy_for_navigation(&moved, destination.page_id(), "https://submit.test/")
                .unwrap()
                .1
                .allowed,
            "accepted navigation preserves its original policy after source retirement"
        );
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ServiceNavigationSnapshot {
    version: u8,
    context: u64,
    agent: oryn_js::worker_wire::AgentId,
    destination: PageId,
    target: Creator,
    url: String,
    fetch: oryn_js::navigation::FormNavigationSource,
    policy: oryn_js::worker_wire::AgentPolicy,
    origin: oryn_js::worker_wire::AgentOrigin,
}
fn service_navigation_mac(key: &[u8; 32], payload: &[u8]) -> [u8; 32] {
    let mut bytes = b"oryn.service-navigation.v1\0".to_vec();
    bytes.extend_from_slice(payload);
    mac(key, &bytes)
}
impl BrowserContext {
    pub(crate) fn issue_service_navigation_authority(
        &self,
        agent: oryn_js::worker_wire::AgentId,
        destination: PageId,
        target: Creator,
        url: &str,
    ) -> Result<oryn_js::navigation::ServiceNavigationAdmission, RuntimeError> {
        self.issue_service_navigation_from(self, agent, destination, target, url)
    }
    pub(crate) fn issue_service_navigation_from(
        &self,
        source_context: &BrowserContext,
        agent: oryn_js::worker_wire::AgentId,
        destination: PageId,
        target: Creator,
        url: &str,
    ) -> Result<oryn_js::navigation::ServiceNavigationAdmission, RuntimeError> {
        let invalid =
            || RuntimeError::ProtocolViolation("invalid service navigation admission".into());
        let bootstrap = source_context
            .worker_bootstrap(agent)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        if bootstrap.kind != oryn_js::worker_wire::AgentKind::Service || url.len() > 65536 {
            return Err(invalid());
        }
        let url = url::Url::parse(url).map_err(|_| invalid())?.to_string();
        let Creator::Window {
            target: window,
            document_generation,
            stream_epoch,
        } = &target
        else {
            return Err(invalid());
        };
        let (_, partition, origin) =
            self.window_worker_creator(destination, window, *document_generation, *stream_epoch)?;
        let target_profile = self
            .context_store()
            .service_coordinator()
            .map_err(|_| invalid())?;
        let source_profile = source_context
            .context_store()
            .service_coordinator()
            .map_err(|_| invalid())?;
        if !std::sync::Arc::ptr_eq(&target_profile, &source_profile)
            || crate::service_registry::stable_partition(&partition).map_err(|_| invalid())?
                != crate::service_registry::stable_partition(&bootstrap.partition)
                    .map_err(|_| invalid())?
        {
            return Err(invalid());
        }
        // The target is a newly service-created root or an already authorized
        // same-origin WindowClient; a service cannot mint foreign target rights.
        if origin.authority != bootstrap.origin.authority {
            return Err(invalid());
        }
        let mut ancestor_urls = bootstrap.cookie_ancestor_urls.clone();
        ancestor_urls.insert(
            0,
            if bootstrap.origin.opaque {
                "about:blank".into()
            } else {
                bootstrap.script_url.clone()
            },
        );
        let fetch = oryn_js::navigation::FormNavigationSource {
            user_activation: false,
            source_domain_key: Some(bootstrap.origin.authority.clone()),
            origin_key: bootstrap.origin.authority.clone(),
            url: bootstrap.script_url.clone(),
            referrer_policy: bootstrap.referrer_policy.clone(),
            top_level_url: bootstrap
                .top_level_url
                .clone()
                .unwrap_or_else(|| bootstrap.script_url.clone()),
            ancestor_urls,
            opaque_origin: bootstrap.origin.opaque,
        };
        // Worker CSP snapshots store referrer policy separately. A local
        // successor inherits that policy container as well as its CSP list.
        let mut policy = bootstrap.policy;
        policy
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("referrer-policy"));
        policy
            .headers
            .push(("referrer-policy".into(), bootstrap.referrer_policy.clone()));
        let snapshot = ServiceNavigationSnapshot {
            version: 1,
            context: self.inner.id,
            agent,
            destination,
            target: target.clone(),
            url,
            fetch,
            policy,
            origin: bootstrap.origin,
        };
        let payload = serde_json::to_vec(&snapshot).map_err(|_| invalid())?;
        if payload.len() > MAX_AUTHORITY_BYTES / 4 {
            return Err(invalid());
        }
        let bytes = serde_json::to_vec(&Sealed {
            mac: service_navigation_mac(self.form_key()?, &payload),
            payload,
        })
        .map_err(|_| invalid())?;
        if bytes.len() > MAX_AUTHORITY_BYTES {
            return Err(invalid());
        }
        source_context
            .worker_bootstrap(agent)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        self.window_worker_creator(destination, window, *document_generation, *stream_epoch)?;
        Ok(oryn_js::navigation::ServiceNavigationAdmission {
            authority: oryn_js::navigation::ServiceNavigationAuthority(bytes),
            target,
        })
    }
    pub(crate) fn service_navigation_source(
        &self,
        authority: &oryn_js::navigation::ServiceNavigationAuthority,
        destination: PageId,
        target: &Creator,
        url: &str,
    ) -> Result<oryn_js::navigation::FormNavigationSource, RuntimeError> {
        Ok(self
            .open_service_navigation(authority, destination, target, url)?
            .fetch)
    }
    pub(crate) fn service_navigation_policy_source(
        &self,
        authority: &oryn_js::navigation::ServiceNavigationAuthority,
        destination: PageId,
        target: &Creator,
        url: &str,
    ) -> Result<
        (
            oryn_js::worker_wire::AgentOrigin,
            oryn_js::worker_wire::AgentPolicy,
        ),
        RuntimeError,
    > {
        let snapshot = self.open_service_navigation(authority, destination, target, url)?;
        Ok((snapshot.origin, snapshot.policy))
    }
    fn open_service_navigation(
        &self,
        authority: &oryn_js::navigation::ServiceNavigationAuthority,
        destination: PageId,
        target: &Creator,
        url: &str,
    ) -> Result<ServiceNavigationSnapshot, RuntimeError> {
        let invalid = || {
            RuntimeError::ProtocolViolation("invalid native service navigation authority".into())
        };
        if authority.0.len() > MAX_AUTHORITY_BYTES {
            return Err(invalid());
        }
        let sealed: Sealed = serde_json::from_slice(&authority.0).map_err(|_| invalid())?;
        let expected = service_navigation_mac(self.form_key()?, &sealed.payload);
        let mut difference = 0u8;
        for (a, b) in expected.iter().zip(sealed.mac) {
            difference |= *a ^ b;
        }
        if difference != 0 {
            return Err(invalid());
        }
        let snapshot: ServiceNavigationSnapshot =
            serde_json::from_slice(&sealed.payload).map_err(|_| invalid())?;
        let url = url::Url::parse(url).map_err(|_| invalid())?.to_string();
        if snapshot.version != 1
            || snapshot.context != self.inner.id
            || snapshot.destination != destination
            || &snapshot.target != target
            || snapshot.url != url
        {
            return Err(invalid());
        }
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = target
        else {
            return Err(invalid());
        };
        self.window_worker_creator(destination, target, *document_generation, *stream_epoch)?;
        // Accepted source metadata survives service task completion. It does not
        // authorize a replacement target or another destination URL.
        Ok(snapshot)
    }
}

impl BrowserContext {
    pub(crate) fn service_navigation_to_script(
        &self,
        page: PageId,
        admission: oryn_js::navigation::ServiceNavigationAdmission,
        url: &str,
    ) -> Result<oryn_js::navigation::ScriptNavigationAdmission, RuntimeError> {
        let source =
            self.open_service_navigation(&admission.authority, page, &admission.target, url)?;
        let snapshot = ScriptSnapshot {
            version: 1,
            context: self.inner.id,
            source: page,
            destination: page,
            source_creator: Creator::Agent(source.agent),
            target_creator: admission.target.clone(),
            destination_url: source.url,
            fetch: source.fetch,
            service_source: Some(admission.authority),
            javascript_policy: None,
            javascript_origin: None,
        };
        let payload = serde_json::to_vec(&snapshot)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if payload.len() > MAX_AUTHORITY_BYTES / 2 {
            return Err(RuntimeError::ProtocolViolation(
                "service script navigation source exceeds bound".into(),
            ));
        }
        let bytes = serde_json::to_vec(&Sealed {
            mac: script_mac(self.form_key()?, &payload),
            payload,
        })
        .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if bytes.len() > MAX_AUTHORITY_BYTES {
            return Err(RuntimeError::ProtocolViolation(
                "service script navigation authority exceeds bound".into(),
            ));
        }
        Ok(oryn_js::navigation::ScriptNavigationAdmission {
            authority: oryn_js::navigation::ScriptNavigationAuthority(bytes),
            target: admission.target,
        })
    }
    pub(crate) fn script_navigation_local_policy(
        &self,
        admission: &oryn_js::navigation::ScriptNavigationAdmission,
        page: PageId,
        url: &str,
    ) -> Result<Option<oryn_js::worker_wire::AgentPolicy>, RuntimeError> {
        self.script_navigation_source(&admission.authority, page, &admission.target, url)?;
        let sealed: Sealed = serde_json::from_slice(&admission.authority.0)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        let source: ScriptSnapshot = serde_json::from_slice(&sealed.payload)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        source
            .service_source
            .as_ref()
            .map(|authority| {
                self.service_navigation_policy_source(authority, page, &admission.target, url)
                    .map(|(_, policy)| policy)
            })
            .transpose()
    }
}

/// A document capability is distinct from a form load capability. Deriving an
/// inert Document copies origin/settings, but does not inherit active sandbox/CSP.
#[derive(serde::Serialize, serde::Deserialize)]
struct DocumentSnapshot {
    version: u8,
    source: Snapshot,
    origin_key: String,
    #[serde(default)]
    relaxed_domain: Option<String>,
    inert: bool,
    document_url: String,
    headers: Vec<(String, String)>,
    ancestors: Vec<oryn_js::message_wire::WindowTarget>,
}
impl BrowserContext {
    fn seal_document_source(
        &self,
        snapshot: &DocumentSnapshot,
    ) -> Result<oryn_dom::DocumentProvenance, RuntimeError> {
        let payload = serde_json::to_vec(snapshot)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        if payload.len() > MAX_AUTHORITY_BYTES / 4 {
            return Err(RuntimeError::ProtocolViolation(
                "document provenance capacity".into(),
            ));
        }
        let mut signed = b"oryn.document-source.v1\0".to_vec();
        signed.extend_from_slice(&payload);
        let capability = serde_json::to_vec(&Sealed {
            mac: mac(self.form_key()?, &signed),
            payload,
        })
        .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        if capability.len() > MAX_AUTHORITY_BYTES {
            return Err(RuntimeError::ProtocolViolation(
                "sealed document provenance capacity".into(),
            ));
        }
        Ok(oryn_dom::DocumentProvenance {
            capability,
            origin: snapshot.source.origin.clone(),
            origin_key: snapshot.origin_key.clone(),
            inert: snapshot.inert,
        })
    }
    fn open_document_source(
        &self,
        _page: PageId,
        provenance: &oryn_dom::DocumentProvenance,
    ) -> Result<DocumentSnapshot, RuntimeError> {
        let invalid = || RuntimeError::ProtocolViolation("invalid document provenance".into());
        if provenance.capability.len() > MAX_AUTHORITY_BYTES {
            return Err(invalid());
        }
        let sealed: Sealed =
            serde_json::from_slice(&provenance.capability).map_err(|_| invalid())?;
        let mut signed = b"oryn.document-source.v1\0".to_vec();
        signed.extend_from_slice(&sealed.payload);
        let expected = mac(self.form_key()?, &signed);
        let mut difference = 0;
        for (a, b) in expected.iter().zip(sealed.mac) {
            difference |= *a ^ b;
        }
        if difference != 0 {
            return Err(invalid());
        }
        let snapshot: DocumentSnapshot =
            serde_json::from_slice(&sealed.payload).map_err(|_| invalid())?;
        if snapshot.version != 1
            || snapshot.source.context != self.inner.id
            || snapshot.origin_key != provenance.origin_key
            || snapshot.source.origin != provenance.origin
            || snapshot.inert != provenance.inert
        {
            return Err(invalid());
        }
        Ok(snapshot)
    }
    pub(crate) fn capture_document_source(
        &self,
        page: PageId,
        creator: Creator,
        url: &str,
        referrer: &str,
    ) -> Result<oryn_dom::DocumentProvenance, RuntimeError> {
        let authority = self.issue_form_authority_with_metadata(
            page,
            creator.clone(),
            Some(referrer),
            Some(url),
        )?;
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "document source is not Window".into(),
            ));
        };
        let (owner, _, origin) =
            self.window_worker_creator(page, &target, document_generation, stream_epoch)?;
        let domain_identity = self
            .native_document_domain_identity(&owner)
            .map_err(RuntimeError::ContextService)?;
        let relaxed_domain = domain_identity.relaxed_domain().map(str::to_owned);
        let ancestors = {
            let policies =
                self.inner.worker_policies.lock().map_err(|_| {
                    RuntimeError::ContextService("document ancestry poisoned".into())
                })?;
            let mut cursor = owner;
            let mut seen = std::collections::BTreeSet::new();
            let mut ancestors = Vec::new();
            while seen.insert(cursor.clone()) {
                let Some(parent) = policies.activation_parents.get(&cursor) else {
                    break;
                };
                if let crate::agent_registry::AgentOwner::Window {
                    page: owner_page,
                    target,
                    ..
                } = parent
                    && *owner_page == page
                {
                    ancestors.push(target.clone());
                }
                cursor = parent.clone();
            }
            ancestors
        };
        let mut source = self.open_form_authority(&authority, page)?;
        source.fetch.user_activation = false;
        source.fetch.source_domain_key = Some(domain_identity.security_token_key());
        self.seal_document_source(&DocumentSnapshot {
            version: 1,
            source,
            origin_key: origin.authority,
            relaxed_domain,
            inert: false,
            document_url: url.into(),
            headers: self
                .window_worker_policy(page, &target, document_generation, stream_epoch)?
                .headers,
            ancestors,
        })
    }
    pub(crate) fn derive_inert_document_source(
        &self,
        page: PageId,
        provenance: &oryn_dom::DocumentProvenance,
        url: &str,
    ) -> Result<oryn_dom::DocumentProvenance, RuntimeError> {
        let mut snapshot = self.open_document_source(page, provenance)?;
        if url.len() > 65536 {
            return Err(RuntimeError::ProtocolViolation(
                "inert document URL capacity".into(),
            ));
        }
        let url = url::Url::parse(url)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        // Constructors either preserve their source URL or use about:blank.
        if url.as_str() != "about:blank" && url.as_str() != snapshot.document_url {
            return Err(RuntimeError::ProtocolViolation(
                "inert document URL does not derive from source".into(),
            ));
        }
        snapshot.inert = true;
        snapshot.source.fetch.user_activation = false;
        snapshot.headers.clear();
        snapshot.document_url = url.to_string();
        snapshot.source.policy = crate::csp::CspPolicySet::new(url.as_str());
        snapshot.source.fetch.referrer_policy =
            crate::referrer::ReferrerPolicy::default().as_str().into();
        self.seal_document_source(&snapshot)
    }
    pub(crate) fn live_document_fetch_source(
        &self,
        page: PageId,
        creator: &Creator,
        provenance: &oryn_dom::DocumentProvenance,
    ) -> Result<oryn_js::navigation::FormNavigationSource, RuntimeError> {
        let snapshot = self.open_document_source(page, provenance)?;
        if snapshot.inert || snapshot.source.source != page || &snapshot.source.creator != creator {
            return Err(RuntimeError::ProtocolViolation(
                "fetch source is not the current native Document".into(),
            ));
        }
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "fetch source requires Window".into(),
            ));
        };
        let (_, _, origin) =
            self.window_worker_creator(page, target, *document_generation, *stream_epoch)?;
        if snapshot.origin_key != origin.authority {
            return Err(RuntimeError::ProtocolViolation(
                "fetch source origin identity changed".into(),
            ));
        }
        Ok(snapshot.source.fetch)
    }
    pub(crate) fn document_source_domain_identity(
        &self,
        page: PageId,
        provenance: &oryn_dom::DocumentProvenance,
    ) -> Result<oryn_js::origin::DomainAccessIdentity, RuntimeError> {
        let snapshot = self.open_document_source(page, provenance)?;
        let origin =
            SecurityOrigin::from_broker_authority(&snapshot.source.origin, &snapshot.origin_key)
                .map_err(|error| RuntimeError::ProtocolViolation(error.into()))?;
        let mut identity = oryn_js::origin::DomainAccessIdentity::new(origin);
        if let Some(domain) = snapshot.relaxed_domain {
            identity.set_domain(&domain).map_err(|error| {
                RuntimeError::ProtocolViolation(format!(
                    "invalid signed document domain: {error:?}"
                ))
            })?;
        }
        Ok(identity)
    }
    pub(crate) fn document_form_authority(
        &self,
        page: PageId,
        provenance: &oryn_dom::DocumentProvenance,
    ) -> Result<FormNavigationAuthority, RuntimeError> {
        let mut snapshot = self.open_document_source(page, provenance)?;
        snapshot.source.fetch.user_activation = !snapshot.inert
            && self
                .navigation_source_activation(snapshot.source.source, &snapshot.source.creator)?;
        snapshot.source.destination = page;
        self.seal_form_authority(&snapshot.source)
    }
    pub(crate) fn document_target_navigation_allowed(
        &self,
        page: PageId,
        provenance: &oryn_dom::DocumentProvenance,
        target: &Creator,
    ) -> Result<bool, RuntimeError> {
        let snapshot = self.open_document_source(page, provenance)?;
        let Creator::Window {
            target: target_window,
            document_generation,
            stream_epoch,
        } = target
        else {
            return Err(RuntimeError::ProtocolViolation(
                "document navigation target is not Window".into(),
            ));
        };
        let (target_owner, _, _) =
            self.window_worker_creator(page, target_window, *document_generation, *stream_epoch)?;
        if snapshot.inert {
            return Ok(true);
        } // null source navigable and an empty sandbox flag set
        let Creator::Window {
            target: source_window,
            document_generation: source_generation,
            stream_epoch: source_epoch,
        } = &snapshot.source.creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "document source Window missing".into(),
            ));
        };
        if self
            .window_worker_creator(
                snapshot.source.source,
                source_window,
                *source_generation,
                *source_epoch,
            )
            .is_ok()
        {
            return self.native_target_navigation_allowed(
                snapshot.source.source,
                &snapshot.source.creator,
                page,
                target,
            );
        }
        // A retired Document keeps its own sandbox, origin and navigable identity.
        // No later document's activation or policy is borrowed.
        if snapshot.source.source == page && source_window == target_window {
            return Ok(true);
        }
        let descendant = {
            let policies = self.inner.worker_policies.lock().map_err(|_| {
                RuntimeError::ContextService("document target ancestry poisoned".into())
            })?;
            let mut cursor = target_owner;
            let mut seen = std::collections::BTreeSet::new();
            let mut found = false;
            while seen.insert(cursor.clone()) {
                let Some(parent) = policies.activation_parents.get(&cursor) else {
                    break;
                };
                if matches!(parent,crate::agent_registry::AgentOwner::Window{page:owner_page,target,..}if *owner_page==snapshot.source.source&&target==source_window)
                {
                    found = true;
                    break;
                }
                cursor = parent.clone();
            }
            found
        };
        if descendant {
            return Ok(true);
        }
        if snapshot.source.source == page && snapshot.ancestors.contains(target_window) {
            if !matches!(target_window, oryn_js::message_wire::WindowTarget::Root) {
                return Ok(true);
            }
            return Ok(snapshot
                .source
                .policy
                .sandbox()
                .is_none_or(|flags| flags.allow_top_navigation));
        }
        Ok(snapshot.source.policy.sandbox().is_none())
    }
}

impl BrowserContext {
    pub(crate) fn install_document_source_metadata(
        &self,
        page: PageId,
        creator: Creator,
        url: &str,
        referrer: &str,
        meta_csp: Option<&str>,
    ) -> Result<oryn_dom::DocumentProvenance, RuntimeError> {
        let source = self.capture_document_source(page, creator.clone(), url, referrer)?;
        let mut snapshot = self.open_document_source(page, &source)?;
        if let Some(value) = meta_csp {
            if value.len() > 65536 {
                return Err(RuntimeError::ProtocolViolation(
                    "document metadata capacity".into(),
                ));
            }
            if !snapshot.source.policy.contains_meta(value) {
                snapshot.source.policy.install_meta(value);
            }
        }
        // Allocate and authenticate the new snapshot before changing parent policy.
        let prepared = self.seal_document_source(&snapshot)?;
        let Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "document source Window missing".into(),
            ));
        };
        let (owner, _, _) =
            self.window_worker_creator(page, &target, document_generation, stream_epoch)?;
        self.install_realm_metadata(&owner, meta_csp, Some(referrer))?;
        Ok(prepared)
    }
}

impl BrowserContext {
    pub(crate) fn document_popup_source(
        &self,
        page: PageId,
        source: &oryn_dom::DocumentProvenance,
    ) -> Result<(Creator, String, crate::csp::CspPolicySet), RuntimeError> {
        let snapshot = self.open_document_source(page, source)?;
        Ok((
            snapshot.source.creator,
            snapshot.source.origin,
            snapshot.source.policy,
        ))
    }
    pub(crate) fn new_named_popup_from_document(
        &self,
        page: PageId,
        source: &oryn_dom::DocumentProvenance,
        noopener: bool,
        target_name: String,
    ) -> Result<PageHandle, RuntimeError> {
        self.new_named_popup_from_document_with_popup(page, source, noopener, target_name, false)
    }

    pub(crate) fn new_named_popup_from_document_with_popup(
        &self,
        page: PageId,
        source: &oryn_dom::DocumentProvenance,
        noopener: bool,
        target_name: String,
        is_popup: bool,
    ) -> Result<PageHandle, RuntimeError> {
        let snapshot = self.open_document_source(page, source)?;
        if target_name.len() > 65536 {
            return Err(RuntimeError::ProtocolViolation(
                "document popup name capacity".into(),
            ));
        }
        if snapshot
            .source
            .policy
            .sandbox()
            .is_some_and(|flags| !flags.allow_popups)
        {
            return Err(RuntimeError::ContextService(
                "document sandbox denies popup".into(),
            ));
        }
        let Creator::Window { target, .. } = &snapshot.source.creator else {
            return Err(RuntimeError::ProtocolViolation(
                "document settings Window missing".into(),
            ));
        };
        let target_name = if noopener || target_name.eq_ignore_ascii_case("_blank") {
            String::new()
        } else {
            target_name
        };
        let origin =
            SecurityOrigin::from_broker_authority(&snapshot.source.origin, &snapshot.origin_key)
                .map_err(|error| RuntimeError::ContextService(error.into()))?;
        let policy = snapshot.source.policy.initial_auxiliary_policy();
        let isolation =
            crate::agent_isolation::DocumentIsolationPolicy::from_headers(&snapshot.headers);
        let permissions = snapshot
            .headers
            .iter()
            .filter(|(name, _)| {
                name.eq_ignore_ascii_case("permissions-policy")
                    || name.eq_ignore_ascii_case("origin-agent-cluster")
            })
            .cloned()
            .collect();
        self.new_popup_with_policy(
            (!noopener).then_some(page),
            None,
            Some(InitialPopupPolicy {
                is_popup,
                relaxed_domain: snapshot.relaxed_domain,
                sandbox_navigator: !noopener && snapshot.source.policy.sandbox().is_some(),
                target_name,
                source_page: page,
                source_origin: snapshot.origin_key,
                creator: snapshot.source.creator.clone(),
                target: target.clone(),
                origin,
                policy,
                isolation,
                permissions,
            }),
        )
    }
}

impl BrowserContext {
    pub(crate) fn document_popup_relationship_policy(
        &self,
        page: PageId,
        source: &oryn_dom::DocumentProvenance,
        url: Option<&str>,
        requested_noopener: bool,
    ) -> Result<(bool, bool), RuntimeError> {
        let snapshot = self.open_document_source(page, source)?;
        let top = self.security_origin(page)?;
        let force_blank = self.page_isolation_policy(page)?.same_origin_opener
            && snapshot.origin_key != top.security_token_key();
        let mut noopener = requested_noopener || force_blank;
        if let Some(url) = url.filter(|url| url.starts_with("blob:")) {
            let blob = self
                .inner
                .worker_policies
                .lock()
                .map_err(|_| {
                    RuntimeError::ContextService("document blob relationship poisoned".into())
                })?
                .blobs
                .get(url)
                .cloned();
            if let Some(blob) = blob {
                self.retained_worker_authority(&blob.owner)
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                let same_site = if blob.origin.authority == top.security_token_key() {
                    true
                } else if top.is_opaque() || blob.origin.serialized == "null" {
                    false
                } else {
                    url::Url::parse(&blob.origin.serialized)
                        .ok()
                        .and_then(|url| crate::cookies::SchemefulSite::from_url(&url))
                        .zip(
                            url::Url::parse(&top.serialize())
                                .ok()
                                .and_then(|url| crate::cookies::SchemefulSite::from_url(&url)),
                        )
                        .is_some_and(|(a, b)| a == b)
                };
                noopener |= !same_site;
            }
        }
        Ok((noopener, force_blank))
    }
}

#[cfg(test)]
mod document_provenance_tests {
    use super::*;
    use oryn_js::message_wire::{MessageRequest, WindowTarget};
    #[test]
    fn sealed_fetch_source_is_exact_live_owner_bound_and_does_not_publish_url() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://source.test/start".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let snapshot = context
            .capture_document_source(page, creator.clone(), "https://source.test/next", "origin")
            .unwrap();
        let fetch = context
            .live_document_fetch_source(page, &creator, &snapshot)
            .unwrap();
        assert_eq!(fetch.url, "https://source.test/next");
        assert_eq!(
            context
                .window_worker_policy(page, &WindowTarget::Root, 1, 0)
                .unwrap()
                .url,
            "https://source.test/start"
        );
        let stale = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 2,
            stream_epoch: 0,
        };
        assert!(
            context
                .live_document_fetch_source(page, &stale, &snapshot)
                .is_err()
        );
        let foreign = context.new_page().page_id();
        assert!(
            context
                .live_document_fetch_source(foreign, &creator, &snapshot)
                .is_err()
        );
        let mut forged = snapshot.clone();
        forged.capability[0] ^= 1;
        assert!(
            context
                .live_document_fetch_source(page, &creator, &forged)
                .is_err()
        );
        let inert = context
            .derive_inert_document_source(page, &snapshot, "https://source.test/next")
            .unwrap();
        assert!(
            context
                .live_document_fetch_source(page, &creator, &inert)
                .is_err()
        );
    }
    #[test]
    fn document_snapshot_does_not_freeze_activation_before_navigation_acceptance() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://source.test/path".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let source = context
            .capture_document_source(page, creator.clone(), "https://source.test/path", "origin")
            .unwrap();
        let owner = context
            .window_worker_creator(page, &WindowTarget::Root, 1, 0)
            .unwrap()
            .0;
        let issue = || {
            context
                .issue_document_script_navigation_authority(
                    page,
                    &source,
                    creator.clone(),
                    "https://source.test/next",
                    None,
                )
                .unwrap()
        };
        let before = issue();
        context.record_native_activation(owner.clone()).unwrap();
        let during = issue();
        assert!(
            !context
                .script_navigation_source(&before, page, &creator, "https://source.test/next")
                .unwrap()
                .user_activation
        );
        assert!(
            context
                .script_navigation_source(&during, page, &creator, "https://source.test/next")
                .unwrap()
                .user_activation
        );
        assert!(context.transient_native_activation(&owner, true).unwrap());
        let after = issue();
        assert!(
            !context
                .script_navigation_source(&after, page, &creator, "https://source.test/next")
                .unwrap()
                .user_activation
        );
        assert!(
            context
                .script_navigation_source(&during, page, &creator, "https://source.test/next")
                .unwrap()
                .user_activation
        );
    }
    #[test]
    fn selected_link_authority_keeps_retired_document_source_and_does_not_apply_form_action() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(
                page,
                "https://source.test/path".into(),
                &[(
                    "content-security-policy".into(),
                    "form-action 'none'".into(),
                )],
            )
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let source = context
            .capture_document_source(
                page,
                creator.clone(),
                "https://source.test/path",
                "unsafe-url",
            )
            .unwrap();
        let destination = context.new_popup_from_creator(page, &creator).unwrap();
        let target = context
            .native_initial_document_creator(destination.page_id())
            .unwrap();
        context
            .native_root_target_name(
                destination.page_id(),
                &target,
                Some("retained-target".into()),
            )
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let local = BTreeMap::from([(page, Vec::new()), (destination.page_id(), Vec::new())]);
        assert_eq!(
            context
                .named_familiar_document_navigation_target(page, &source, "retained-target", &local)
                .unwrap(),
            Some((destination.page_id(), None))
        );
        assert!(
            context
                .named_familiar_navigation_target_with_local(
                    page,
                    &creator,
                    "retained-target",
                    &local
                )
                .is_err(),
            "live Window selection cannot impersonate a retired source"
        );
        assert_eq!(
            context
                .named_familiar_document_navigation_target(page, &source, "absent-target", &local)
                .unwrap(),
            None
        );
        let mut forged_origin = source.clone();
        forged_origin.origin_key = "borrowed-live-origin".into();
        assert!(
            context
                .named_familiar_document_navigation_target(
                    page,
                    &forged_origin,
                    "retained-target",
                    &local
                )
                .is_err()
        );
        let authority = context
            .issue_document_script_navigation_authority(
                destination.page_id(),
                &source,
                target.clone(),
                "https://target.test/next",
                Some("no-referrer"),
            )
            .unwrap();
        let fetch = context
            .script_navigation_source(
                &authority,
                destination.page_id(),
                &target,
                "https://target.test/next",
            )
            .unwrap();
        assert_eq!(fetch.url, "https://source.test/path");
        assert_eq!(fetch.origin_key, source.origin_key);
        assert_eq!(fetch.referrer_policy, "no-referrer");
        assert!(
            context
                .script_navigation_source(&authority, page, &target, "https://target.test/next")
                .is_err()
        );
        let mut forged = source.clone();
        forged.capability[0] ^= 1;
        assert!(
            context
                .issue_document_script_navigation_authority(
                    destination.page_id(),
                    &forged,
                    target,
                    "https://target.test/next",
                    None
                )
                .is_err()
        );
    }
    #[test]
    fn derived_inert_source_has_own_policy_and_survives_source_retirement_without_forgery() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(
                page,
                "https://source.test/path".into(),
                &[(
                    "content-security-policy".into(),
                    "form-action 'none'".into(),
                )],
            )
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let active = context
            .capture_document_source(
                page,
                creator.clone(),
                "https://source.test/path",
                "no-referrer",
            )
            .unwrap();
        let inert = context
            .derive_inert_document_source(page, &active, "about:blank")
            .unwrap();
        assert_eq!(active.origin_key, inert.origin_key);
        assert!(inert.inert);
        let active_form = context.document_form_authority(page, &active).unwrap();
        let inert_form = context.document_form_authority(page, &inert).unwrap();
        assert!(
            !context
                .form_policy_for_navigation(&active_form, page, "https://receipt.test/")
                .unwrap()
                .1
                .allowed
        );
        assert!(
            context
                .form_policy_for_navigation(&inert_form, page, "https://receipt.test/")
                .unwrap()
                .1
                .allowed
        );
        assert_eq!(
            context
                .form_fetch_source(&inert_form, page)
                .unwrap()
                .referrer_policy,
            crate::referrer::ReferrerPolicy::default().as_str()
        );
        assert!(
            context
                .derive_inert_document_source(page, &active, "https://forged.test/")
                .is_err()
        );
        let mut forged = inert.clone();
        forged.origin_key = "https://forged.test".into();
        assert!(context.document_form_authority(page, &forged).is_err());
        let mut forged = inert.clone();
        forged.inert = false;
        assert!(context.document_form_authority(page, &forged).is_err());
        let mut forged = inert.clone();
        forged.capability[0] ^= 1;
        assert!(context.document_form_authority(page, &forged).is_err());
        assert!(
            Browser::new()
                .new_context()
                .document_form_authority(page, &inert)
                .is_err()
        );
        context
            .handle_message_wire(
                page,
                MessageRequest::RetireRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        assert!(
            context
                .capture_document_source(page, creator, "https://source.test/path", "no-referrer")
                .is_err()
        );
        let cloned = context
            .derive_inert_document_source(page, &inert, "about:blank")
            .unwrap();
        assert_eq!(
            context
                .form_fetch_source(
                    &context.document_form_authority(page, &cloned).unwrap(),
                    page
                )
                .unwrap(),
            context.form_fetch_source(&inert_form, page).unwrap()
        );
    }
    #[test]
    fn metadata_prepares_document_snapshot_before_policy_commit() {
        let context = Browser::new().new_context();
        let page = context.new_page().page_id();
        context
            .install_document_security(page, "https://source.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let old = context
            .capture_document_source(page, creator.clone(), "https://source.test/", "origin")
            .unwrap();
        let failed = context.install_document_source_metadata(
            page,
            creator.clone(),
            "https://source.test/",
            "origin",
            Some(&"x".repeat(65537)),
        );
        assert!(failed.is_err());
        assert!(
            context
                .form_policy_for_navigation(
                    &context.issue_form_authority(page, creator.clone()).unwrap(),
                    page,
                    "https://receipt.test/"
                )
                .unwrap()
                .1
                .allowed
        );
        let next = context
            .install_document_source_metadata(
                page,
                creator,
                "https://source.test/",
                "no-referrer",
                Some("form-action 'none'"),
            )
            .unwrap();
        assert!(
            !context
                .form_policy_for_navigation(
                    &context.document_form_authority(page, &next).unwrap(),
                    page,
                    "https://receipt.test/"
                )
                .unwrap()
                .1
                .allowed
        );
        assert!(
            context
                .form_policy_for_navigation(
                    &context.document_form_authority(page, &old).unwrap(),
                    page,
                    "https://receipt.test/"
                )
                .unwrap()
                .1
                .allowed,
            "earlier immutable snapshots are not mutated"
        );
    }
}
