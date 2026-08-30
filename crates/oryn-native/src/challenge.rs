//! High-precision, deterministic site-challenge classification.
//!
//! A generic denial status, challenge-like prose, an empty observation, or a
//! native runtime diagnostic is never sufficient to confirm a challenge. The
//! execution-domain policy consumes only [`ChallengeDisposition::Confirmed`].

use serde::{Deserialize, Serialize};

pub const CHALLENGE_DETECTOR_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeDisposition {
    #[default]
    NotDetected,
    Suspected,
    Confirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeSignalSource {
    ResponseStatus,
    ResponseHeader,
    DomMarker,
    ResourceUrl,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeSignal {
    pub source: ChallengeSignalSource,
    /// A detector-owned identifier. Page-authored text is never copied here.
    pub marker: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeAssessment {
    pub detector_version: u16,
    pub disposition: ChallengeDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signals: Vec<ChallengeSignal>,
}

impl Default for ChallengeAssessment {
    fn default() -> Self {
        Self {
            detector_version: CHALLENGE_DETECTOR_VERSION,
            disposition: ChallengeDisposition::NotDetected,
            provider: None,
            signals: Vec::new(),
        }
    }
}

impl ChallengeAssessment {
    pub fn is_confirmed(&self) -> bool {
        self.disposition == ChallengeDisposition::Confirmed
    }
}

#[derive(Clone, Copy)]
struct ProviderRule {
    provider: &'static str,
    resource_markers: &'static [&'static str],
    control_markers: &'static [&'static str],
    header_markers: &'static [(&'static str, &'static str)],
}

const PROVIDERS: &[ProviderRule] = &[
    ProviderRule {
        provider: "cloudflare",
        resource_markers: &["challenges.cloudflare.com"],
        control_markers: &["cf-turnstile-response", "cf-chl-"],
        header_markers: &[("cf-mitigated", "challenge")],
    },
    ProviderRule {
        provider: "recaptcha",
        resource_markers: &["google.com/recaptcha", "gstatic.com/recaptcha"],
        control_markers: &["g-recaptcha-response", "recaptcha-token"],
        header_markers: &[],
    },
    ProviderRule {
        provider: "hcaptcha",
        resource_markers: &["hcaptcha.com/1/api.js"],
        control_markers: &["h-captcha-response", "hcaptcha-response"],
        header_markers: &[],
    },
    ProviderRule {
        provider: "arkose",
        resource_markers: &["arkoselabs.com", "funcaptcha"],
        control_markers: &["fc-token", "arkose-token"],
        header_markers: &[],
    },
    ProviderRule {
        provider: "datadome_perimeterx",
        resource_markers: &["datadome.co/captcha", "perimeterx"],
        control_markers: &["datadome", "px-captcha", "_px3"],
        header_markers: &[("x-datadome", "")],
    },
];

fn signal(source: ChallengeSignalSource, marker: impl Into<String>) -> ChallengeSignal {
    ChallengeSignal {
        source,
        marker: marker.into(),
    }
}

/// Classify accepted navigation metadata and raw HTML before script execution.
pub fn assess_challenge(
    status: u16,
    headers: &[(String, String)],
    html: &str,
) -> ChallengeAssessment {
    let lower_html = html.to_ascii_lowercase();
    for rule in PROVIDERS {
        let resource = rule
            .resource_markers
            .iter()
            .find(|marker| lower_html.contains(**marker));
        let control = rule
            .control_markers
            .iter()
            .find(|marker| lower_html.contains(**marker));
        let header = rule.header_markers.iter().find(|(name, value)| {
            headers.iter().any(|(candidate, candidate_value)| {
                candidate.eq_ignore_ascii_case(name)
                    && (value.is_empty() || candidate_value.to_ascii_lowercase().contains(*value))
            })
        });
        if let (Some(resource), Some(control)) = (resource, control) {
            return ChallengeAssessment {
                detector_version: CHALLENGE_DETECTOR_VERSION,
                disposition: ChallengeDisposition::Confirmed,
                provider: Some(rule.provider.into()),
                signals: vec![
                    signal(ChallengeSignalSource::ResourceUrl, *resource),
                    signal(ChallengeSignalSource::DomMarker, *control),
                ],
            };
        }
        if let (Some((name, _)), Some(control)) = (header, control) {
            return ChallengeAssessment {
                detector_version: CHALLENGE_DETECTOR_VERSION,
                disposition: ChallengeDisposition::Confirmed,
                provider: Some(rule.provider.into()),
                signals: vec![
                    signal(ChallengeSignalSource::ResponseHeader, *name),
                    signal(ChallengeSignalSource::DomMarker, *control),
                ],
            };
        }
    }

    let mut signals = Vec::new();
    if matches!(status, 401 | 403 | 429) {
        signals.push(signal(
            ChallengeSignalSource::ResponseStatus,
            status.to_string(),
        ));
    }
    if [
        "verify you are human",
        "checking your browser",
        "complete the security check",
        "captcha",
    ]
    .iter()
    .any(|marker| lower_html.contains(marker))
    {
        signals.push(signal(
            ChallengeSignalSource::Text,
            "generic_challenge_language",
        ));
    }
    ChallengeAssessment {
        detector_version: CHALLENGE_DETECTOR_VERSION,
        disposition: if signals.is_empty() {
            ChallengeDisposition::NotDetected
        } else {
            ChallengeDisposition::Suspected
        },
        provider: None,
        signals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirms_only_paired_provider_evidence() {
        let assessment = assess_challenge(
            403,
            &[],
            r#"<script src="https://challenges.cloudflare.com/turnstile/v0/api.js"></script><input name="cf-turnstile-response">"#,
        );
        assert_eq!(assessment.disposition, ChallengeDisposition::Confirmed);
        assert_eq!(assessment.provider.as_deref(), Some("cloudflare"));
    }

    #[test]
    fn generic_denial_and_prose_are_only_suspected() {
        let assessment = assess_challenge(403, &[], "Please verify you are human");
        assert_eq!(assessment.disposition, ChallengeDisposition::Suspected);
        assert!(!assessment.is_confirmed());
    }

    #[test]
    fn native_failures_are_not_detector_inputs() {
        let assessment = assess_challenge(200, &[], "Unexpected token; empty semantic observation");
        assert_eq!(assessment, ChallengeAssessment::default());
    }
}
