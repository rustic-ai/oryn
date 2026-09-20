#![cfg(unix)]

use oryn_js::worker_wire::{AgentOrigin, AgentPolicy, WorkerCredentials};
use oryn_native::{
    agent_script::ScriptAuthority,
    context_service::{DocumentIsolationPolicy, EmbedderPolicy, RelatedRootAdmission},
    csp::ScriptHashReport,
};
use std::cell::RefCell;

#[test]
fn external_script_authority_can_receive_hash_report_payloads() {
    let received = RefCell::new(Vec::new());
    let report = |reports: Vec<ScriptHashReport>| {
        for report in reports {
            received
                .borrow_mut()
                .push((report.endpoint, report.document_url, report.body));
        }
    };
    let origin = AgentOrigin {
        serialized: "https://reports.test".into(),
        authority: "https://reports.test".into(),
        opaque: false,
    };
    let policy = AgentPolicy::default();
    let authority = ScriptAuthority {
        hash_report: Some(&report),
        worker_destination: "worker",
        coep_report: None,
        report: None,
        cookie_ancestor_urls: &[],
        referrer_source: None,
        origin: &origin,
        policy: &policy,
        script_url: "https://reports.test/worker.js",
        referrer_policy: "no-referrer",
        top_level_url: Some("https://reports.test/"),
        credentials: WorkerCredentials::SameOrigin,
    };
    authority.hash_report.unwrap()(vec![ScriptHashReport {
        endpoint: "https://reports.test/collector".into(),
        document_url: authority.script_url.into(),
        body: b"{\"type\":\"csp-hash\"}".to_vec(),
    }]);
    assert_eq!(
        received.into_inner(),
        vec![(
            "https://reports.test/collector".to_string(),
            "https://reports.test/worker.js".to_string(),
            b"{\"type\":\"csp-hash\"}".to_vec(),
        )]
    );
}

#[test]
fn external_related_root_policy_preserves_wire_shape() {
    // Synthetic wire data exercises the public payload; it is never admitted
    // to a native authority registry or used to issue an execution grant.
    let route = serde_json::json!({
        "process_session": 1, "environment": 2, "generation": 3,
        "grant": vec![0_u8; 32],
    });
    for embedder in [
        EmbedderPolicy::None,
        EmbedderPolicy::RequireCorp,
        EmbedderPolicy::Credentialless,
    ] {
        let isolation = DocumentIsolationPolicy {
            same_origin_opener: true,
            embedder,
            same_origin_permission: false,
        };
        let admission = RelatedRootAdmission {
            direct: true,
            route: serde_json::from_value(route.clone()).unwrap(),
            actor_route: serde_json::from_value(route.clone()).unwrap(),
            policy: None,
            isolation,
        };
        let wire = serde_json::to_value(&admission).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "direct": true, "route": route, "actor_route": route,
                "policy": null, "isolation": {
                    "same_origin_opener": true,
                    "embedder": match embedder {
                        EmbedderPolicy::None => "None",
                        EmbedderPolicy::RequireCorp => "RequireCorp",
                        EmbedderPolicy::Credentialless => "Credentialless",
                    },
                    "same_origin_permission": false,
                },
            })
        );
        let decoded: RelatedRootAdmission = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(decoded.isolation, isolation);
        assert_eq!(decoded.route, admission.route);
        assert_eq!(decoded.actor_route, admission.actor_route);
        let mut unknown = wire;
        unknown
            .as_object_mut()
            .unwrap()
            .insert("authority_override".into(), true.into());
        assert!(serde_json::from_value::<RelatedRootAdmission>(unknown).is_err());
    }
}
