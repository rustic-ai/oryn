//! Native frame resource results shared by portable and parent-backed loaders.
//! Socket serialization stays in context_service; frame consumers do not need
//! a Unix transport merely to carry document bytes or a no-commit outcome.
#[cfg(any(test, feature = "v8-host"))]
#[derive(Debug, Clone)]
pub(crate) enum FrameResourceResponse {
    EmbeddedResourceUnavailable,
    PageResourceNoCommit {
        reason: String,
    },
    PageBinaryResource {
        timing: Option<oryn_js::platform::ResourceTimingMetadata>,
        url: String,
        body_base64: String,
        content_type: Option<String>,
        destination: String,
    },
    PageResource {
        inherited_encoding: Option<String>,
        response_headers: Vec<(String, String)>,
        raw_body_base64: Option<String>,
        request_state: Option<oryn_common::navigation_request::NavigationRequestState>,
        timing: Option<Box<oryn_js::platform::ResourceTimingMetadata>>,
        content_language: Option<String>,
        stream_id: Option<u64>,
        native_origin: Option<oryn_js::worker_wire::AgentOrigin>,
        effective_post: Option<oryn_js::navigation::PostResource>,
        worker_policy_token: Option<String>,
        url: String,
        source: String,
        encoding: String,
        modals_allowed: bool,
        destination: String,
        content_type: Option<String>,
        document_referrer: String,
        referrer_policy: String,
        refresh_url: Option<String>,
    },
}

// Charge retained allocations, including capacity reserved beyond the visible
// payload. Fixed inline storage is charged once by size_of::<Self>(); boxed
// document timing contributes its pointee storage in addition to nested allocations.
#[cfg(any(test, feature = "v8-host"))]
impl FrameResourceResponse {
    pub(crate) fn retained_bytes(&self) -> Result<usize, String> {
        let mut size = RetainedBytes(std::mem::size_of::<Self>());
        match self {
            Self::EmbeddedResourceUnavailable => {}
            Self::PageResourceNoCommit { reason } => size.string(reason)?,
            Self::PageBinaryResource {
                timing,
                url,
                body_base64,
                content_type,
                destination,
            } => {
                for value in [url, body_base64, destination] {
                    size.string(value)?;
                }
                size.optional(content_type)?;
                size.timing(timing.as_ref())?;
            }
            Self::PageResource {
                inherited_encoding,
                response_headers,
                raw_body_base64,
                request_state,
                timing,
                content_language,
                stream_id: _,
                native_origin,
                effective_post,
                worker_policy_token,
                url,
                source,
                encoding,
                modals_allowed: _,
                destination,
                content_type,
                document_referrer,
                referrer_policy,
                refresh_url,
            } => {
                for value in [
                    url,
                    source,
                    encoding,
                    destination,
                    document_referrer,
                    referrer_policy,
                ] {
                    size.string(value)?;
                }
                for value in [
                    inherited_encoding,
                    raw_body_base64,
                    content_language,
                    worker_policy_token,
                    content_type,
                    refresh_url,
                ] {
                    size.optional(value)?;
                }
                size.vector(response_headers)?;
                for (name, value) in response_headers {
                    size.string(name)?;
                    size.string(value)?;
                }
                if let Some(state) = request_state {
                    // Preserve validation of the frozen navigation provenance.
                    state.retained_bytes().map_err(str::to_owned)?;
                    size.optional(&state.initiator_url)?;
                    size.optional(&state.referrer)?;
                    size.string(&state.referrer_policy)?;
                }
                if let Some(origin) = native_origin {
                    size.string(&origin.serialized)?;
                    size.string(&origin.authority)?;
                }
                if let Some(post) = effective_post {
                    size.vector(&post.bytes)?;
                    size.string(&post.content_type)?;
                }
                if timing.is_some() {
                    size.add(std::mem::size_of::<oryn_js::platform::ResourceTimingMetadata>())?;
                }
                size.timing(timing.as_deref())?;
            }
        }
        Ok(size.0)
    }
}
#[cfg(any(test, feature = "v8-host"))]
struct RetainedBytes(usize);
#[cfg(any(test, feature = "v8-host"))]
impl RetainedBytes {
    fn add(&mut self, value: usize) -> Result<(), String> {
        self.0 = self
            .0
            .checked_add(value)
            .ok_or_else(|| "frame response retained byte accounting overflow".to_owned())?;
        Ok(())
    }
    fn string(&mut self, value: &String) -> Result<(), String> {
        self.add(value.capacity())
    }
    fn optional(&mut self, value: &Option<String>) -> Result<(), String> {
        if let Some(value) = value {
            self.string(value)?;
        }
        Ok(())
    }
    fn vector<T>(&mut self, value: &Vec<T>) -> Result<(), String> {
        self.add(
            value
                .capacity()
                .checked_mul(std::mem::size_of::<T>())
                .ok_or_else(|| "frame response retained byte accounting overflow".to_owned())?,
        )
    }
    fn hops(&mut self, hops: &Vec<oryn_common::transport_timing::TimingHop>) -> Result<(), String> {
        self.vector(hops)?;
        for hop in hops {
            self.string(&hop.url)?;
            self.vector(&hop.allow_origin)?;
            for origin in &hop.allow_origin {
                self.string(origin)?;
            }
        }
        Ok(())
    }
    fn timing(
        &mut self,
        timing: Option<&oryn_js::platform::ResourceTimingMetadata>,
    ) -> Result<(), String> {
        if let Some(timing) = timing {
            self.string(&timing.origin)?;
            self.string(&timing.content_type)?;
            self.hops(&timing.hops)?;
            self.vector(&timing.server_timing)?;
            for metric in &timing.server_timing {
                self.string(&metric.name)?;
                self.string(&metric.description)?;
            }
            if let Some(transport) = &timing.transport {
                self.string(&transport.next_hop_protocol)?;
                if let Some(exposure) = &transport.exposure {
                    self.string(&exposure.origin)?;
                    self.hops(&exposure.hops)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(all(unix, any(test, feature = "v8-host")))]
impl TryFrom<crate::context_service::ContextServiceResponse> for FrameResourceResponse {
    type Error = String;
    fn try_from(
        response: crate::context_service::ContextServiceResponse,
    ) -> Result<Self, Self::Error> {
        use crate::context_service::ContextServiceResponse as Wire;
        Ok(match response {
            Wire::EmbeddedResourceUnavailable => Self::EmbeddedResourceUnavailable,
            Wire::PageResourceNoCommit { reason } => Self::PageResourceNoCommit { reason },
            Wire::PageBinaryResource {
                timing,
                url,
                body_base64,
                content_type,
                destination,
            } => Self::PageBinaryResource {
                timing,
                url,
                body_base64,
                content_type,
                destination,
            },
            Wire::PageResource {
                inherited_encoding,
                response_headers,
                raw_body_base64,
                request_state,
                timing,
                content_language,
                stream_id,
                native_origin,
                effective_post,
                worker_policy_token,
                url,
                source,
                encoding,
                modals_allowed,
                destination,
                content_type,
                document_referrer,
                referrer_policy,
                refresh_url,
            } => Self::PageResource {
                inherited_encoding,
                response_headers,
                raw_body_base64,
                request_state,
                timing,
                content_language,
                stream_id,
                native_origin,
                effective_post,
                worker_policy_token,
                url,
                source,
                encoding,
                modals_allowed,
                destination,
                content_type,
                document_referrer,
                referrer_policy,
                refresh_url,
            },
            _ => return Err("unexpected native frame resource response".into()),
        })
    }
}

// Raster object/embed responses have no nested Document or child policy.
pub(crate) fn embedded_document_mime(content_type: Option<&str>) -> bool {
    let essence = content_type
        .unwrap_or("text/html")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    essence == "text/html"
        || oryn_dom::xml::is_text_mime(&essence)
        || matches!(essence.as_str(), "text/xml" | "application/xml")
        || essence.ends_with("+xml")
        || oryn_dom::xml::media_document_tag(&essence).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binary_and_no_commit_charge_reserved_body_and_reason_capacity() {
        let mut body = String::with_capacity(4096);
        body.push_str("AQ==");
        let capacity = body.capacity();
        let response = FrameResourceResponse::PageBinaryResource {
            timing: None,
            url: String::new(),
            body_base64: body,
            content_type: None,
            destination: String::new(),
        };
        assert_eq!(
            response.retained_bytes().unwrap(),
            std::mem::size_of::<FrameResourceResponse>() + capacity
        );
        let mut reason = String::with_capacity(512);
        reason.push_str("no-content");
        let capacity = reason.capacity();
        assert_eq!(
            FrameResourceResponse::PageResourceNoCommit { reason }
                .retained_bytes()
                .unwrap(),
            std::mem::size_of::<FrameResourceResponse>() + capacity
        );
    }
    #[test]
    fn timing_charges_both_response_and_transport_redirect_exposure() {
        use oryn_common::transport_timing::{TimingExposure, TimingHop, TransportTiming};
        let mut timing = oryn_js::platform::ResourceTimingMetadata {
            server_timing: vec![],
            response_end_micros: None,
            navigation_phases: None,
            producer_clock_micros: None,
            decoded_body_size: None,
            origin: String::new(),
            hops: vec![],
            transport: None,
            allow_details: false,
            response_status: 200,
            response_start_micros: 0,
            encoded_body_size: None,
            content_type: String::new(),
        };
        let mut baseline = RetainedBytes(0);
        baseline.timing(Some(&timing)).unwrap();
        assert_eq!(baseline.0, 0);
        timing.hops.push(TimingHop {
            url: "https://redirect.test/".into(),
            allow_origin: vec!["https://source.test".into()],
        });
        let mut response_only = RetainedBytes(0);
        response_only.timing(Some(&timing)).unwrap();
        assert!(response_only.0 > 0);
        timing.transport = Some(TransportTiming {
            exposure: Some(TimingExposure {
                origin: "https://source.test".into(),
                hops: timing.hops.clone(),
                allowed: false,
            }),
            next_hop_protocol: "h2".into(),
            ..Default::default()
        });
        timing.server_timing.push(oryn_js::server_timing::Metric {
            name: "cache".into(),
            description: "retained detail".into(),
            duration: 1.,
        });
        let mut all = RetainedBytes(0);
        all.timing(Some(&timing)).unwrap();
        assert!(all.0 > response_only.0);
        let nested_charge = all.0;
        let mut document = FrameResourceResponse::PageResource {
            inherited_encoding: None,
            response_headers: Vec::new(),
            raw_body_base64: None,
            request_state: None,
            timing: None,
            content_language: None,
            stream_id: None,
            native_origin: None,
            effective_post: None,
            worker_policy_token: None,
            url: String::new(),
            source: String::new(),
            encoding: String::new(),
            modals_allowed: false,
            destination: String::new(),
            content_type: None,
            document_referrer: String::new(),
            referrer_policy: String::new(),
            refresh_url: None,
        };
        let empty_charge = document.retained_bytes().unwrap();
        assert_eq!(empty_charge, std::mem::size_of::<FrameResourceResponse>());
        if let FrameResourceResponse::PageResource { timing: slot, .. } = &mut document {
            *slot = Some(Box::new(timing));
        }
        assert_eq!(
            document.retained_bytes().unwrap() - empty_charge,
            std::mem::size_of::<oryn_js::platform::ResourceTimingMetadata>() + nested_charge
        );
    }
    #[test]
    fn overflow_is_an_error_not_a_wrapped_or_saturated_charge() {
        let mut size = RetainedBytes(usize::MAX);
        assert!(size.add(1).is_err());
        assert_eq!(size.0, usize::MAX);
    }
    #[test]
    fn document_classification_preserves_mime_essence_and_binary_distinction() {
        for mime in [
            None,
            Some("text/html"),
            Some(" Text/HTML ; charset=utf-8"),
            Some("application/xhtml+xml"),
            Some("image/svg+xml"),
            Some("text/plain"),
        ] {
            assert!(embedded_document_mime(mime), "{mime:?}");
        }
        for mime in [
            Some("image/png"),
            Some("image/jpeg"),
            Some("application/octet-stream"),
        ] {
            assert!(!embedded_document_mime(mime), "{mime:?}");
        }
    }
    #[cfg(unix)]
    #[test]
    fn wire_document_timing_keeps_json_shape_and_transfers_allocation() {
        use crate::context_service::ContextServiceResponse as Wire;
        let mut wire: Wire = serde_json::from_value(serde_json::json!({
            "result": "page_resource",
            "url": "https://timing.test/",
            "source": "body",
            "encoding": "utf-8",
            "destination": "frame"
        }))
        .unwrap();
        let Wire::PageResource { timing, .. } = &mut wire else {
            panic!("document response expected")
        };
        assert!(timing.is_none());
        *timing = Some(Box::new(oryn_js::platform::ResourceTimingMetadata {
            server_timing: Vec::new(),
            response_end_micros: None,
            navigation_phases: None,
            producer_clock_micros: None,
            decoded_body_size: None,
            origin: "https://timing.test".into(),
            hops: Vec::new(),
            transport: None,
            allow_details: true,
            response_status: 200,
            response_start_micros: 123,
            encoded_body_size: Some(4),
            content_type: "text/html".into(),
        }));
        let value = serde_json::to_value(&wire).unwrap();
        assert_eq!(value["timing"]["origin"], "https://timing.test");
        assert_eq!(value["timing"]["response_start_micros"], 123);
        let decoded: Wire = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
        let allocation = match &wire {
            Wire::PageResource {
                timing: Some(timing),
                ..
            } => timing.as_ref() as *const _,
            _ => panic!("timing missing"),
        };
        let FrameResourceResponse::PageResource {
            timing: Some(timing),
            ..
        } = FrameResourceResponse::try_from(wire).unwrap()
        else {
            panic!("timing lost in adapter")
        };
        assert_eq!(timing.as_ref() as *const _, allocation);
        assert_eq!(timing.response_start_micros, 123);
        assert_eq!(timing.encoded_body_size, Some(4));
    }

    #[cfg(unix)]
    #[test]
    fn wire_adapter_preserves_original_document_and_request_custody() {
        use crate::context_service::ContextServiceResponse as Wire;
        let request = oryn_common::navigation_request::NavigationRequestState {
            initiator_url: Some("https://original.test/form".into()),
            opaque_initiator: true,
            referrer: None,
            referrer_policy: "no-referrer".into(),
        };
        let post = oryn_js::navigation::PostResource {
            bytes: vec![0, 255, 3],
            content_type: "application/octet-stream".into(),
        };
        let native = oryn_js::worker_wire::AgentOrigin {
            serialized: "null".into(),
            authority: "opaque-original".into(),
            opaque: true,
        };
        let response = FrameResourceResponse::try_from(Wire::PageResource {
            inherited_encoding: Some("windows-1252".into()),
            response_headers: vec![("content-type".into(), "text/html".into())],
            raw_body_base64: Some("AP8D".into()),
            request_state: Some(request.clone()),
            timing: None,
            content_language: Some("fr".into()),
            stream_id: Some(42),
            native_origin: Some(native.clone()),
            effective_post: Some(post.clone()),
            worker_policy_token: Some("original-policy".into()),
            url: "https://destination.test/final".into(),
            source: "<body>original bytes</body>".into(),
            encoding: "utf-8".into(),
            modals_allowed: true,
            destination: "iframe".into(),
            content_type: Some("text/html".into()),
            document_referrer: "".into(),
            referrer_policy: "no-referrer".into(),
            refresh_url: Some("https://destination.test/next".into()),
        })
        .unwrap();
        let FrameResourceResponse::PageResource {
            request_state,
            effective_post,
            native_origin,
            stream_id,
            worker_policy_token,
            raw_body_base64,
            response_headers,
            inherited_encoding,
            content_language,
            source,
            url,
            encoding,
            modals_allowed,
            destination,
            content_type,
            document_referrer,
            referrer_policy,
            refresh_url,
            timing,
        } = response
        else {
            panic!("document result lost")
        };
        assert_eq!(request_state, Some(request));
        assert_eq!(effective_post, Some(post));
        assert_eq!(native_origin, Some(native));
        assert_eq!(stream_id, Some(42));
        assert_eq!(worker_policy_token.as_deref(), Some("original-policy"));
        assert_eq!(raw_body_base64.as_deref(), Some("AP8D"));
        assert_eq!(
            response_headers,
            vec![("content-type".into(), "text/html".into())]
        );
        assert_eq!(inherited_encoding.as_deref(), Some("windows-1252"));
        assert_eq!(content_language.as_deref(), Some("fr"));
        assert_eq!(source, "<body>original bytes</body>");
        assert_eq!(url, "https://destination.test/final");
        assert_eq!(encoding, "utf-8");
        assert!(modals_allowed);
        assert_eq!(destination, "iframe");
        assert_eq!(content_type.as_deref(), Some("text/html"));
        assert!(document_referrer.is_empty());
        assert_eq!(referrer_policy, "no-referrer");
        assert_eq!(
            refresh_url.as_deref(),
            Some("https://destination.test/next")
        );
        assert!(timing.is_none());
        assert!(FrameResourceResponse::try_from(Wire::PageClosed).is_err());
        assert!(
            matches!(FrameResourceResponse::try_from(Wire::PageResourceNoCommit{reason:"no-content".into()}).unwrap(),FrameResourceResponse::PageResourceNoCommit{reason}if reason=="no-content")
        );
        assert!(
            matches!(FrameResourceResponse::try_from(Wire::PageBinaryResource{timing:None,url:"https://image.test/a".into(),body_base64:"AP8D".into(),content_type:Some("image/png".into()),destination:"object".into()}).unwrap(),FrameResourceResponse::PageBinaryResource{body_base64,..}if body_base64=="AP8D")
        );
    }
}
