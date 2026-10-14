// Exact native/web failure distinction for retained import loading. Resource
// policy denial is a failed import; worker quota and owner cancellation are not
// successful empty stylesheet responses and cannot disappear in string errors.
struct StylesheetFetchOutcome {
    response: Option<RetainedStylesheetResponse>,
    failure: Option<crate::RuntimeError>,
}
impl StylesheetFetchOutcome {
    fn response(response: RetainedStylesheetResponse) -> Self { Self { response: Some(response), failure: None } }
}
fn classify_stylesheet_fetch_error(error: crate::RuntimeError)
    -> Result<StylesheetFetchOutcome, crate::RuntimeError> {
    match error {
        crate::RuntimeError::NavigationCancelled => Ok(StylesheetFetchOutcome { response: None, failure: None }),
        error @ (crate::RuntimeError::Network(_) | crate::RuntimeError::CertificateRejected(_))
            => Ok(StylesheetFetchOutcome { response: None, failure: Some(error) }),
        error => Err(error),
    }
}
fn stylesheet_budget_runtime_error(error: HostError) -> crate::RuntimeError {
    match error {
        HostError::LimitExceeded { resource } => crate::RuntimeError::WorkerLimitExceeded { resource },
        error => crate::RuntimeError::Script(error.to_string()),
    }
}
fn stylesheet_fetch_host_error(error: crate::RuntimeError) -> HostError {
    match error {
        crate::RuntimeError::WorkerLimitExceeded { resource } => HostError::LimitExceeded { resource },
        crate::RuntimeError::NavigationCancelled => HostError::CachedAuthorityExpired,
        error => HostError::Execution(error.to_string()),
    }
}
// Only actual web fetch/certificate failures become failed imports. Protocol,
// identity, quota, native allocation and unsupported-capability errors preserve
// their original diagnostics rather than falsely becoming loaded/empty CSS.
