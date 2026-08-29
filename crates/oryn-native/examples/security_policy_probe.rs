use std::collections::BTreeMap;

use oryn_common::v3::SemanticAction;
use oryn_native::{
    ActionPolicy, Browser, ConfirmationToken, ConsequentialAction, ContextOptions, PolicyDecision,
    PolicyRequest, PolicyRule,
    runtime::{NativeAction, RuntimeError},
};
use serde::Serialize;

#[derive(Serialize)]
struct ProbeResult {
    schema_version: u32,
    suite: &'static str,
    confirmation_exact_scope: bool,
    confirmation_one_use: bool,
    all_sensitive_actions_default_confirmed: bool,
    origin_policy_denial: bool,
    secret_handle_origin_scoped: bool,
    secret_revocation: bool,
    password_observation_redacted: bool,
    secret_absent_from_policy_audit: bool,
    structured_security_audit: bool,
    download_quarantined: bool,
    download_path_sanitized: bool,
    download_size_limited: bool,
    audit_records: usize,
    outcome: Outcome,
}

#[derive(Serialize)]
struct Outcome {
    status: &'static str,
}

fn request(action: ConsequentialAction, detail: &str) -> PolicyRequest {
    PolicyRequest {
        action,
        source_origin: "https://app.example".into(),
        target_origin: None,
        detail: Some(detail.into()),
    }
}

fn confirmed(context: &oryn_native::BrowserContext, request: PolicyRequest) -> ConfirmationToken {
    context.prepare_confirmation(request)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let context = Browser::new().new_context();

    let exact = request(ConsequentialAction::Clipboard, "copy summary");
    let token = confirmed(&context, exact.clone());
    let confirmation_exact_scope = context.authorize(
        request(ConsequentialAction::Clipboard, "different data"),
        Some(token),
    ) == PolicyDecision::ConfirmationRequired;

    let once = request(ConsequentialAction::FileUpload, "invoice.pdf");
    let once_token = confirmed(&context, once.clone());
    let confirmation_one_use = context.authorize(once.clone(), Some(once_token))
        == PolicyDecision::Allowed
        && context.authorize(once, Some(once_token)) == PolicyDecision::ConfirmationRequired;

    let sensitive = [
        ConsequentialAction::FileUpload,
        ConsequentialAction::Download,
        ConsequentialAction::Clipboard,
        ConsequentialAction::CredentialFill,
        ConsequentialAction::ProtectedRequest,
        ConsequentialAction::UserBrowserDelegation,
    ];
    let all_sensitive_actions_default_confirmed = sensitive.into_iter().all(|action| {
        context.authorize(request(action, "probe"), None) == PolicyDecision::ConfirmationRequired
    });

    let mut rules = BTreeMap::new();
    rules.insert(
        ConsequentialAction::CrossOriginNavigation,
        PolicyRule::Allow,
    );
    rules.insert(
        ConsequentialAction::Download,
        PolicyRule::RequireConfirmation,
    );
    let denied_options = ContextOptions {
        action_policy: ActionPolicy {
            rules,
            denied_origins: ["https://blocked.example".into()].into_iter().collect(),
            allowed_origins: Default::default(),
            max_download_bytes: 4,
        },
        ..ContextOptions::default()
    };
    let denied_context = Browser::new().new_context_with_options(denied_options);
    let origin_policy_denial = denied_context.authorize(
        PolicyRequest {
            action: ConsequentialAction::CrossOriginNavigation,
            source_origin: "https://app.example".into(),
            target_origin: Some("https://blocked.example".into()),
            detail: None,
        },
        None,
    ) == PolicyDecision::Denied;

    let page = context.new_page();
    page.load_html(
        "https://app.example/",
        r#"<label>Password<input id="password" type="password"></label>"#,
    )
    .await?;
    let target = page
        .resolve_selector("#password", SemanticAction::Type)
        .await?;
    let handle = context.import_secret(
        "https://app.example",
        "probe credential",
        b"never-observe-this".to_vec(),
    );
    let fill_request = request(ConsequentialAction::CredentialFill, "opaque secret fill");
    page.fill_secret(target, handle, Some(confirmed(&context, fill_request)))
        .await?;
    let observation = page.observe().await?;
    let serialized = serde_json::to_string(&observation)?;
    let password_observation_redacted =
        serialized.contains("[REDACTED]") && !serialized.contains("never-observe-this");

    let other_page = context.new_page();
    other_page
        .load_html(
            "https://other.example/",
            r#"<input id="password" type="password">"#,
        )
        .await?;
    let other_target = other_page
        .resolve_selector("#password", SemanticAction::Type)
        .await?;
    let other_fill = PolicyRequest {
        action: ConsequentialAction::CredentialFill,
        source_origin: "https://other.example".into(),
        target_origin: None,
        detail: Some("opaque secret fill".into()),
    };
    let secret_handle_origin_scoped = matches!(
        other_page
            .fill_secret(other_target, handle, Some(confirmed(&context, other_fill)))
            .await,
        Err(RuntimeError::Secret(_))
    );
    let secret_revocation = context.revoke_secret(handle)
        && matches!(
            page.fill_secret(
                target,
                handle,
                Some(confirmed(
                    &context,
                    request(ConsequentialAction::CredentialFill, "opaque secret fill")
                ))
            )
            .await,
            Err(RuntimeError::Secret(_))
        );

    let source_url = "https://app.example/download/report";
    let download_request = PolicyRequest {
        action: ConsequentialAction::Download,
        source_origin: "https://app.example".into(),
        target_origin: None,
        detail: Some(source_url.into()),
    };
    let descriptor = context.quarantine_download(
        source_url,
        "../../report.txt",
        "text/plain",
        b"report".to_vec(),
        Some(confirmed(&context, download_request)),
    )?;
    let download_quarantined = descriptor.quarantined
        && context
            .downloads()
            .iter()
            .any(|item| item.id == descriptor.id)
        && context.release_download(descriptor.id)? == b"report";
    let download_path_sanitized = descriptor.suggested_filename == "report.txt";

    let too_large_url = "https://app.example/download/large";
    let too_large_request = PolicyRequest {
        action: ConsequentialAction::Download,
        source_origin: "https://app.example".into(),
        target_origin: None,
        detail: Some(too_large_url.into()),
    };
    let download_size_limited = matches!(
        denied_context.quarantine_download(
            too_large_url,
            "large.bin",
            "application/octet-stream",
            vec![0; 5],
            Some(confirmed(&denied_context, too_large_request)),
        ),
        Err(RuntimeError::Download(_))
    );

    page.execute(NativeAction {
        target,
        action: SemanticAction::Clear,
        value: None,
    })
    .await?;
    page.close().await?;
    other_page.close().await?;

    let context_audit = context.security_audit();
    let denied_audit = denied_context.security_audit();
    let audit_json = serde_json::to_string(&(context_audit.clone(), denied_audit.clone()))?;
    let secret_absent_from_policy_audit = !audit_json.contains("never-observe-this");
    let structured_security_audit = !context_audit.is_empty()
        && !denied_audit.is_empty()
        && context_audit
            .windows(2)
            .all(|records| records[0].sequence < records[1].sequence)
        && denied_audit
            .windows(2)
            .all(|records| records[0].sequence < records[1].sequence);
    let audit_records = context_audit.len() + denied_audit.len();
    let passed = confirmation_exact_scope
        && confirmation_one_use
        && all_sensitive_actions_default_confirmed
        && origin_policy_denial
        && secret_handle_origin_scoped
        && secret_revocation
        && password_observation_redacted
        && secret_absent_from_policy_audit
        && structured_security_audit
        && download_quarantined
        && download_path_sanitized
        && download_size_limited;
    let result = ProbeResult {
        schema_version: 1,
        suite: "oryn2-security-policy-probe",
        confirmation_exact_scope,
        confirmation_one_use,
        all_sensitive_actions_default_confirmed,
        origin_policy_denial,
        secret_handle_origin_scoped,
        secret_revocation,
        password_observation_redacted,
        secret_absent_from_policy_audit,
        structured_security_audit,
        download_quarantined,
        download_path_sanitized,
        download_size_limited,
        audit_records,
        outcome: Outcome {
            status: if passed { "passed" } else { "failed" },
        },
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    if !passed {
        return Err("security policy probe failed".into());
    }
    Ok(())
}
