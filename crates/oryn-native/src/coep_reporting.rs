//! COEP reporting uses committed policy and the existing bounded native report queue.

/// Inputs for response.
pub(crate) struct ResponsePolicyInputs<'a> {
    pub(crate) origin: &'a str,
    pub(crate) opaque: bool,
    pub(crate) target: &'a url::Url,
    pub(crate) cors: bool,
    pub(crate) credentials: bool,
    pub(crate) headers: &'a [(String, String)],
    pub(crate) destination: &'a str,
}
use crate::agent_isolation::{DocumentIsolationPolicy, EmbedderPolicy};
#[derive(Clone, Debug, Default)]
pub(crate) struct Policy {
    document_url: String,
    rules: Vec<Rule>,
}
#[derive(Clone, Debug)]
struct Rule {
    embedder: EmbedderPolicy,
    endpoint: String,
    disposition: &'static str,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub endpoint: String,
    pub document_url: String,
    pub body: Vec<u8>,
}
fn report_url(value: &str) -> String {
    let Ok(mut url) = url::Url::parse(value) else {
        return String::new();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    url.to_string()
}
fn group(value: &str) -> Option<String> {
    crate::agent_isolation::structured_token(value)?;
    let mut quote = false;
    let mut escaped = false;
    let mut start = 0;
    let mut parts = Vec::new();
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quote = !quote
        }
        if !quote && ch == ';' {
            parts.push(&value[start..index]);
            start = index + 1;
        }
    }
    parts.push(&value[start..]);
    let mut result = None;
    for part in parts.into_iter().skip(1) {
        let Some((name, value)) = part.trim_start().split_once('=') else {
            continue;
        };
        if name != "report-to" {
            continue;
        }
        let value = value.trim_end();
        result = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .map(|value| {
                let mut out = String::new();
                let mut chars = value.chars();
                while let Some(ch) = chars.next() {
                    if ch == '\\' {
                        if let Some(ch) = chars.next() {
                            out.push(ch)
                        }
                    } else {
                        out.push(ch)
                    }
                }
                out
            });
    }
    result
}
impl Policy {
    pub(crate) fn new(document_url: &str, headers: &[(String, String)]) -> Self {
        let endpoints = crate::csp::parse_reporting_endpoints(headers, document_url);
        let mut rules = Vec::new();
        for (name, disposition) in [
            ("cross-origin-embedder-policy-report-only", "reporting"),
            ("cross-origin-embedder-policy", "enforce"),
        ] {
            let mut values = headers
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(name));
            let Some((_, value)) = values.next() else {
                continue;
            };
            if values.next().is_some() {
                continue;
            }
            let embedder = match crate::agent_isolation::structured_token(value) {
                Some("require-corp") => EmbedderPolicy::RequireCorp,
                Some("credentialless") => EmbedderPolicy::Credentialless,
                _ => continue,
            };
            if let Some(endpoint) = group(value).and_then(|group| endpoints.get(&group).cloned()) {
                rules.push(Rule {
                    embedder,
                    endpoint,
                    disposition,
                });
            }
        }
        Self {
            document_url: report_url(document_url),
            rules,
        }
    }
    pub(crate) fn response(&self, params: ResponsePolicyInputs<'_>) -> Vec<Report> {
        let ResponsePolicyInputs {
            origin,
            opaque,
            target,
            cors,
            credentials,
            headers,
            destination,
        } = params;

        self.rules
            .iter()
            .filter_map(|rule| {
                let policy = DocumentIsolationPolicy {
                    embedder: rule.embedder,
                    ..Default::default()
                };
                policy
                    .authorize_response(origin, opaque, target, cors, credentials, headers)
                    .is_err()
                    .then(|| self.report(rule, target, "corp", Some(destination)))
            })
            .collect()
    }
    pub(crate) fn frame(
        &self,
        origin: &str,
        opaque: bool,
        target: &url::Url,
        credentials: bool,
        headers: &[(String, String)],
    ) -> Vec<Report> {
        self.rules
            .iter()
            .filter_map(|rule| {
                let policy = DocumentIsolationPolicy {
                    embedder: rule.embedder,
                    ..Default::default()
                };
                if policy
                    .authorize_response(origin, opaque, target, false, credentials, headers)
                    .is_err()
                {
                    Some(self.report(rule, target, "corp", Some("iframe")))
                } else if policy.authorize_frame_response(headers).is_err() {
                    Some(self.report(rule, target, "navigation", None))
                } else {
                    None
                }
            })
            .collect()
    }
    fn report(
        &self,
        rule: &Rule,
        target: &url::Url,
        kind: &str,
        destination: Option<&str>,
    ) -> Report {
        let mut body = serde_json::json!({"type":kind,"blockedURL":report_url(target.as_str()),"disposition":rule.disposition});
        if let Some(destination) = destination {
            body["destination"] = destination.into();
        }
        Report {
            endpoint: rule.endpoint.clone(),
            document_url: self.document_url.clone(),
            body: serde_json::to_vec(
                &serde_json::json!([{"age":0,"type":"coep","url":self.document_url,"body":body}]),
            )
            .unwrap_or_default(),
        }
    }
}
impl crate::runtime::BrowserContext {
    pub(crate) fn deliver_coep_reports(
        &self,
        top_level: &str,
        reports: Vec<Report>,
    ) -> Result<(), crate::runtime::RuntimeError> {
        for report in reports {
            self.queue_native_report_payload(
                None,
                top_level.into(),
                crate::csp::CspReportTarget {
                    url: report.endpoint,
                    format: crate::csp::CspReportFormat::ReportingApi,
                },
                report.document_url,
                "application/reports+json".into(),
                report.body,
            )?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_only_emits_without_granting_or_blocking_and_keeps_full_target_url() {
        let headers = vec![
            (
                "cross-origin-embedder-policy-report-only".into(),
                "require-corp; report-to=\"group\"".into(),
            ),
            (
                "reporting-endpoints".into(),
                "group=\"https://report.test/endpoint\"".into(),
            ),
        ];
        let policy = Policy::new("https://app.test/page?x=1#fragment", &headers);
        let target = url::Url::parse("https://user:pass@foreign.test/image?q=2#fragment").unwrap();
        assert_eq!(
            DocumentIsolationPolicy::from_headers(&headers).embedder,
            EmbedderPolicy::None
        );
        let reports = policy.response(ResponsePolicyInputs {
            origin: "https://app.test",
            opaque: false,
            target: &target,
            cors: false,
            credentials: true,
            headers: &[],
            destination: "image",
        });
        assert_eq!(reports.len(), 1);
        let value: serde_json::Value = serde_json::from_slice(&reports[0].body).unwrap();
        assert_eq!(
            value[0]["body"],
            serde_json::json!({"type":"corp","blockedURL":"https://foreign.test/image?q=2","destination":"image","disposition":"reporting"})
        );
        assert!(
            policy
                .response(ResponsePolicyInputs {
                    origin: "https://app.test",
                    opaque: false,
                    target: &target,
                    cors: true,
                    credentials: true,
                    headers: &[],
                    destination: "image"
                })
                .is_empty()
        );
        assert!(
            policy
                .response(ResponsePolicyInputs {
                    origin: "https://app.test",
                    opaque: false,
                    target: &target,
                    cors: false,
                    credentials: true,
                    headers: &[("cross-origin-resource-policy".into(), "cross-origin".into())],
                    destination: "image"
                })
                .is_empty()
        );
        let cross = policy.frame("https://app.test", false, &target, true, &[]);
        let cross: serde_json::Value = serde_json::from_slice(&cross[0].body).unwrap();
        assert_eq!(cross[0]["body"]["type"], "corp");
        assert_eq!(cross[0]["body"]["destination"], "iframe");
        let same = url::Url::parse("https://app.test/child").unwrap();
        let navigation = policy.frame("https://app.test", false, &same, true, &[]);
        let navigation: serde_json::Value = serde_json::from_slice(&navigation[0].body).unwrap();
        assert_eq!(navigation[0]["body"]["type"], "navigation");
        assert!(navigation[0]["body"].get("destination").is_none());
        assert!(
            policy
                .frame(
                    "https://app.test",
                    false,
                    &same,
                    true,
                    &[("cross-origin-embedder-policy".into(), "require-corp".into())]
                )
                .is_empty()
        );
    }
}
