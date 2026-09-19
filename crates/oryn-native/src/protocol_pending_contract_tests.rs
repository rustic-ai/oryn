//! Regression sources for the collective unbuilt protocol batch. Constants are
//! intentionally read from production; this module does not reserve a version.
use crate::{
    context_service::{ContextServiceRequest as Request, ContextServiceResponse as Response},
    worker::{WorkerRequest, WorkerResponse},
};
use oryn_common::v3::PageId;
use oryn_js::{
    message_wire::WindowTarget,
    worker_wire::{AgentCommand, AgentServiceRequest, Creator},
};
use serde::{Serialize, de::DeserializeOwned};
fn creator() -> Creator {
    Creator::Window {
        target: WindowTarget::Root,
        document_generation: 7,
        stream_epoch: 3,
    }
}
fn roundtrip<T: Serialize + DeserializeOwned>(value: &T) -> serde_json::Value {
    let wire = serde_json::to_value(value).unwrap();
    let decoded: T = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    wire
}
fn required<T: Serialize + DeserializeOwned>(value: &T, tag: &str, name: &str, fields: &[&str]) {
    let wire = roundtrip(value);
    assert_eq!(wire[tag], name);
    for field in fields {
        let mut missing = wire.clone();
        missing.as_object_mut().unwrap().remove(*field);
        assert!(
            serde_json::from_value::<T>(missing).is_err(),
            "{name} accepted missing {field}"
        );
    }
    let mut unknown = wire;
    unknown[tag] = serde_json::json!(format!("{name}_unknown"));
    assert!(serde_json::from_value::<T>(unknown).is_err());
}
#[test]
fn close_and_history_projection_require_original_principal_and_transaction_fields() {
    required(
        &Request::ClosePage {
            source_page_id: PageId(1),
            target_page_id: PageId(2),
            creator: creator(),
            expected: None,
        },
        "operation",
        "close_page",
        &["source_page_id", "target_page_id", "creator"],
    );
    required(
        &Request::ReadScriptCloseState {
            source_page_id: PageId(1),
            target_page_id: PageId(2),
            creator: creator(),
            expected: None,
        },
        "operation",
        "read_script_close_state",
        &["source_page_id", "target_page_id", "creator"],
    );
    for (request, name) in [
        (
            Request::BeginScriptCloseHistory {
                page_id: PageId(1),
                creator: creator(),
                revision: 9,
                count: 2,
            },
            "begin_script_close_history",
        ),
        (
            Request::PublishScriptCloseHistory {
                page_id: PageId(1),
                creator: creator(),
                revision: 9,
                count: 2,
            },
            "publish_script_close_history",
        ),
        (
            Request::AbortScriptCloseHistory {
                page_id: PageId(1),
                creator: creator(),
                revision: 9,
                count: 2,
            },
            "abort_script_close_history",
        ),
    ] {
        required(
            &request,
            "operation",
            name,
            &["page_id", "creator", "revision", "count"],
        );
    }
    required(
        &WorkerRequest::PrepareScriptClose {
            token: 11,
            expected: creator(),
        },
        "command",
        "prepare_script_close",
        &["token", "expected"],
    );
    let proof = crate::context_service::ScriptCloseHistoryProof {
        creator: creator(),
        single_entry: false,
    };
    let wire = roundtrip(&WorkerResponse::ScriptCloseHistory { proof });
    assert_eq!(wire["status"], "script_close_history");
    for field in ["creator", "single_entry"] {
        let mut missing = wire.clone();
        missing["proof"].as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<WorkerResponse>(missing).is_err());
    }
}
#[test]
fn frame_opener_preserves_target_identity_and_prerender_requires_exact_scope() {
    required(
        &Request::NativeWindowOpener {
            page_id: PageId(1),
            creator: creator(),
        },
        "operation",
        "native_window_opener",
        &["page_id", "creator"],
    );
    required(
        &Request::DiscardNativeFrameOpener {
            page_id: PageId(1),
            frame: "child".into(),
        },
        "operation",
        "discard_native_frame_opener",
        &["page_id", "frame"],
    );
    let wire = roundtrip(&Response::WindowOpener {
        opener: Some((PageId(2), WindowTarget::Frame("child".into()))),
    });
    assert_eq!(wire["result"], "window_opener");
    assert_ne!(wire["opener"], serde_json::Value::Null);
    let token: oryn_js::message_wire::PrerenderToken =
        serde_json::from_value(serde_json::to_value([5u8; 32]).unwrap()).unwrap();
    for (request, name) in [
        (
            Request::MarkPrerenderReady {
                page_id: PageId(1),
                token: token.clone(),
                generation: 8,
                epoch: 4,
            },
            "mark_prerender_ready",
        ),
        (
            Request::MarkPrerenderLoaded {
                page_id: PageId(1),
                token: token.clone(),
                generation: 8,
                epoch: 4,
            },
            "mark_prerender_loaded",
        ),
        (
            Request::CancelPrerender {
                page_id: PageId(1),
                token: token.clone(),
                generation: 8,
                epoch: 4,
            },
            "cancel_prerender",
        ),
    ] {
        required(
            &request,
            "operation",
            name,
            &["page_id", "token", "generation", "epoch"],
        );
    }
    let activation = crate::context_service::BlankPrerenderActivation {
        page_id: PageId(2),
        token,
        generation: 8,
        epoch: 4,
        url: "https://example.test/next".into(),
    };
    for (request, name) in [
        (
            Request::ActivateBlankPrerender {
                page_id: PageId(1),
                activation: activation.clone(),
            },
            "activate_blank_prerender",
        ),
        (
            Request::PublishBlankPrerender {
                page_id: PageId(1),
                activation: activation.clone(),
            },
            "publish_blank_prerender",
        ),
        (
            Request::AcknowledgeBlankPrerender {
                page_id: PageId(1),
                activation,
            },
            "acknowledge_blank_prerender",
        ),
    ] {
        required(&request, "operation", name, &["page_id", "activation"]);
        let wire = roundtrip(&request);
        for field in ["page_id", "token", "generation", "epoch", "url"] {
            let mut missing = wire.clone();
            missing["activation"].as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<Request>(missing).is_err());
        }
    }
}

#[test]
fn embedded_selection_and_retained_prerender_policy_have_distinct_wire_contracts() {
    use crate::context_service::{NavigationPolicySource, ResourceInitiator};
    let initiator = ResourceInitiator {
        embedded_type: Some("image/svg+xml".into()),
        request_state: None,
        navigation: Default::default(),
        stylesheet: None,
        creator: creator(),
        prepared_policy: None,
    };
    let wire = roundtrip(&initiator);
    assert_eq!(wire["embedded_type"], "image/svg+xml");
    for invalid in [
        serde_json::json!(true),
        serde_json::json!(17),
        serde_json::json!([]),
    ] {
        let mut bad = wire.clone();
        bad["embedded_type"] = invalid;
        assert!(serde_json::from_value::<ResourceInitiator>(bad).is_err());
    }
    let mut missing = wire;
    missing.as_object_mut().unwrap().remove("embedded_type");
    assert!(
        serde_json::from_value::<ResourceInitiator>(missing)
            .unwrap()
            .embedded_type
            .is_none()
    );
    assert_eq!(
        roundtrip(&Response::EmbeddedResourceUnavailable),
        serde_json::json!({"result":"embedded_resource_unavailable"})
    );
    let token: oryn_js::message_wire::PrerenderToken =
        serde_json::from_value(serde_json::to_value([5u8; 32]).unwrap()).unwrap();
    let policy = NavigationPolicySource::Prerender {
        token,
        generation: 8,
        epoch: 4,
    };
    let wire = roundtrip(&policy);
    // Native retained authority cannot degrade into an InitialAboutBlank or a
    // current Document lookup when a capability/lifetime field is omitted.
    for field in ["token", "generation", "epoch"] {
        let mut missing = wire.clone();
        missing["Prerender"].as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<NavigationPolicySource>(missing).is_err());
    }
}
#[test]
fn result_payloads_keep_their_discriminator_and_separate_outcome() {
    let cases = [
        (
            Response::DeferredFetchState {
                result: Err(oryn_js::fetch_later::DeferredFetchError::Transport(
                    "closed".into(),
                )),
            },
            "deferred_fetch_state",
        ),
        (
            Response::DeferredFetchResult { result: Ok(()) },
            "deferred_fetch_result",
        ),
        (
            Response::NativePlatformResult {
                result: Err("retired".into()),
            },
            "native_platform_result",
        ),
        (
            Response::UserActivationRecorded { result: Ok(()) },
            "user_activation_recorded",
        ),
        (
            Response::TransientUserActivation { result: Ok(false) },
            "transient_user_activation",
        ),
        (
            Response::ClipboardScriptAuthorized { result: Ok(()) },
            "clipboard_script_authorized",
        ),
        (
            Response::ClipboardRead {
                result: Err(oryn_js::clipboard::ClipboardError::Retired),
            },
            "clipboard_read",
        ),
        (
            Response::ClipboardWrite { result: Ok(()) },
            "clipboard_write",
        ),
        (
            Response::MessageWire {
                result: Err("retired".into()),
            },
            "message_wire",
        ),
        (
            Response::WorkerService {
                result: Ok(oryn_js::worker_wire::AgentServiceResponse::Done),
            },
            "worker_service",
        ),
        (
            Response::NavigationControl { result: Ok(()) },
            "navigation_control",
        ),
        (
            Response::DocumentDomainPrepared {
                result: Err("retired".into()),
            },
            "document_domain_prepared",
        ),
    ];
    for (value, tag) in cases {
        required(&value, "result", tag, &["outcome"]);
    }
}
#[test]
fn context_profile_roundtrips_explicit_selection_without_url_rules() {
    let mut options = crate::runtime::ContextOptions::default();
    options.web_features.usermedia_element = crate::feature_profile::UserMediaElementMode::Legacy;
    let wire = roundtrip(&options);
    assert_eq!(wire["web_features"]["usermedia_element"], "legacy");
    required(
        &Request::WebFeatureProfile { page_id: PageId(1) },
        "operation",
        "web_feature_profile",
        &["page_id"],
    );
    required(
        &Response::WebFeatureProfile {
            profile: options.web_features,
        },
        "result",
        "web_feature_profile",
        &["profile"],
    );
    // Context construction retains its documented standard default. This is
    // not permission to accept an old worker/agent handshake after the bump.
    let mut old = wire;
    old.as_object_mut().unwrap().remove("web_features");
    let decoded: crate::runtime::ContextOptions = serde_json::from_value(old).unwrap();
    assert_eq!(decoded.web_features, Default::default());
}
#[test]
fn keepalive_survives_agent_fetch_and_platform_service_frames() {
    let request:oryn_js::worker_wire::AgentFetchRequest=serde_json::from_value(serde_json::json!({"keepalive":true,"id":9,"client_id":"client","url":"https://example.test/","method":"POST","headers":[],"body":[1,2],"destination":"","mode":"cors","credentials":"include"})).unwrap();
    let command = AgentCommand::Fetch(request);
    let wire = roundtrip(&command);
    assert_eq!(wire["Fetch"]["keepalive"], true);
    let http = oryn_js::platform::PlatformHttpRequest {
        upgrade_mixed_image: false,
        fetch: Some(oryn_js::platform::FetchOptions {
            keepalive: true,
            privileged_range: false,
            cache: oryn_js::platform::FetchCacheMode::NoStore,
            mode: "cors".into(),
            redirect: "follow".into(),
            integrity: String::new(),
        }),
        use_cors_preflight: false,
        method: "POST".into(),
        url: "https://example.test/".into(),
        headers: vec![],
        body: vec![1, 2],
        credentials: "include".into(),
        initiator_url: None,
        referrer_policy: None,
    };
    let request = crate::agent_ipc::ServiceRequest {
        id: 19,
        request: AgentServiceRequest::PlatformSubmit {
            operation_id: oryn_js::platform::HostOperationId(17),
            request: oryn_js::platform::PlatformHostRequest::HttpStart { request: http },
        },
    };
    let mut bytes = Vec::new();
    crate::agent_ipc::write_frame(&mut bytes, &request).unwrap();
    let restored: crate::agent_ipc::ServiceRequest =
        crate::agent_ipc::read_frame(&mut bytes.as_slice()).unwrap();
    assert_eq!(
        serde_json::to_value(restored).unwrap(),
        serde_json::to_value(request).unwrap()
    );
}
