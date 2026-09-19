//! Popup interaction heuristic: native opener partition, native click principal.
use super::*;
use crate::agent_registry::AgentOwner;
use oryn_js::{message_wire::WindowTarget, worker_wire::WorkerError};
impl BrowserContext {
    pub(crate) fn grant_popup_interaction_storage_access(
        &self,
        owner: &AgentOwner,
    ) -> Result<(), WorkerError> {
        let AgentOwner::Window {
            page,
            target: WindowTarget::Root,
            ..
        } = owner
        else {
            return Ok(());
        };
        let Some(cookies) = self.inner.parent_cookies.as_ref() else {
            return Ok(());
        };
        let captured = self
            .inner
            .group
            .popup_cookie_sites
            .lock()
            .map_err(|_| WorkerError::new("InvalidStateError", "popup partition lock"))?
            .get(page)
            .cloned();
        let Some((opener, top)) = captured else {
            return Ok(());
        };
        // COOP, noopener, closed openers and native opener severing invalidate
        // the relationship. The opener's *current* URL cannot change the
        // partition captured when this auxiliary context was created.
        let (_, current, related) = self
            .native_window_group(*page, opener, None)
            .map_err(|error| WorkerError::new("InvalidStateError", error.to_string()))?;
        if current != Some(opener) || !related {
            return Ok(());
        }
        let (_, origin, _) = self.native_worker_authority(owner)?;
        if origin.opaque {
            return Ok(());
        }
        let top = Url::parse(&top)
            .map_err(|error| WorkerError::new("InvalidStateError", error.to_string()))?;
        let embedded = Url::parse(&origin.serialized)
            .map_err(|error| WorkerError::new("InvalidStateError", error.to_string()))?;
        if !matches!(top.scheme(), "http" | "https")
            || !matches!(embedded.scheme(), "http" | "https")
            || !oryn_js::realm::potentially_trustworthy_origin(&oryn_js::origin::Origin::from_url(
                top.as_str(),
            ))
            || !oryn_js::realm::potentially_trustworthy_origin(&oryn_js::origin::Origin::from_url(
                embedded.as_str(),
            ))
        {
            return Ok(());
        }
        // Recheck native identity immediately before publication. No authored
        // opener/location field or request-supplied origin participates.
        self.native_worker_authority(owner)?;
        cookies
            .grant_storage_access(&top, &embedded)
            .map_err(|error| WorkerError::new("QuotaExceededError", error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::{message_wire::MessageRequest, worker_wire::Creator};
    fn root(context: &BrowserContext, page: PageId, url: &str, generation: u64) -> AgentOwner {
        context
            .install_document_security(page, url.into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                page,
                MessageRequest::RegisterRootDocument {
                    document_generation: generation,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        context
            .window_worker_creator(page, &WindowTarget::Root, generation, 0)
            .unwrap()
            .0
    }
    fn allowed(context: &BrowserContext, top: &str, origin: &str) -> bool {
        context
            .inner
            .parent_cookies
            .as_ref()
            .unwrap()
            .has_storage_access(
                &Url::parse(origin).unwrap(),
                &crate::cookies::CookieRequestContext {
                    top_level_url: Some(top.into()),
                    ancestor_urls: vec![top.into()],
                    ..Default::default()
                },
            )
    }
    #[test]
    fn popup_click_grants_only_frozen_opener_site_and_live_popup_origin() {
        let context = Browser::new().new_context();
        context
            .inner
            .parent_cookies
            .as_ref()
            .unwrap()
            .set_third_party_policy(crate::cookies::ThirdPartyCookiePolicy::Block);
        let opener = context.new_page().page_id();
        root(&context, opener, "https://top.test/", 1);
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let popup = context
            .new_named_popup_from_creator(opener, &creator, false, String::new())
            .unwrap()
            .page_id();
        let owner = root(&context, popup, "https://embed.test/", 1);
        assert!(!allowed(
            &context,
            "https://top.test/",
            "https://embed.test/"
        ));
        assert!(!allowed(
            &context,
            "https://elsewhere.test/",
            "https://embed.test/"
        ));
        root(&context, opener, "https://elsewhere.test/", 2);
        context.record_native_activation(owner.clone()).unwrap();
        assert!(allowed(
            &context,
            "https://top.test/",
            "https://embed.test/"
        ));
        assert!(!allowed(
            &context,
            "https://elsewhere.test/",
            "https://embed.test/"
        ));
        assert!(!allowed(
            &context,
            "https://top.test/",
            "https://other.test/"
        ));
        let next = root(&context, popup, "https://other.test/", 2);
        assert!(context.record_native_activation(owner).is_err());
        assert!(!allowed(
            &context,
            "https://top.test/",
            "https://other.test/"
        ));
        context.record_native_activation(next).unwrap();
        assert!(allowed(
            &context,
            "https://top.test/",
            "https://other.test/"
        ));
    }
    #[test]
    fn ordinary_windows_noopener_and_severed_popup_relationships_do_not_grant() {
        let context = Browser::new().new_context();
        context
            .inner
            .parent_cookies
            .as_ref()
            .unwrap()
            .set_third_party_policy(crate::cookies::ThirdPartyCookiePolicy::Block);
        let opener = context.new_page().page_id();
        let first = root(&context, opener, "https://top.test/", 1);
        context.record_native_activation(first).unwrap();
        assert!(!allowed(
            &context,
            "https://other.test/",
            "https://top.test/"
        ));
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let isolated = context
            .new_named_popup_from_creator(opener, &creator, true, String::new())
            .unwrap()
            .page_id();
        let isolated_owner = root(&context, isolated, "https://isolated.test/", 1);
        context.record_native_activation(isolated_owner).unwrap();
        assert!(!allowed(
            &context,
            "https://top.test/",
            "https://isolated.test/"
        ));
        let popup = context
            .new_named_popup_from_creator(opener, &creator, false, String::new())
            .unwrap()
            .page_id();
        let owner = root(&context, popup, "https://embed.test/", 1);
        context
            .inner
            .group
            .pages
            .lock()
            .unwrap()
            .get_mut(&popup)
            .unwrap()
            .descriptor
            .page
            .opener_page_id = None;
        context.record_native_activation(owner).unwrap();
        assert!(!allowed(
            &context,
            "https://top.test/",
            "https://embed.test/"
        ));
    }
}
