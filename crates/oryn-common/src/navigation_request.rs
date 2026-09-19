//! Request provenance retained by a session-history document state.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationRequestState {
    pub initiator_url: Option<String>,
    pub opaque_initiator: bool,
    pub referrer: Option<String>,
    pub referrer_policy: String,
}
impl NavigationRequestState {
    /// Validate serialized custody before copying it into retained history.
    pub fn retained_bytes(&self) -> Result<usize, &'static str> {
        if !matches!(
            self.referrer_policy.as_str(),
            "no-referrer"
                | "no-referrer-when-downgrade"
                | "same-origin"
                | "origin"
                | "strict-origin"
                | "origin-when-cross-origin"
                | "strict-origin-when-cross-origin"
                | "unsafe-url"
        ) {
            return Err("invalid history referrer policy");
        }
        let mut bytes = std::mem::size_of::<Self>() + self.referrer_policy.len();
        for value in [&self.initiator_url, &self.referrer].into_iter().flatten() {
            if value.len() > 64 * 1024
                || value.chars().any(|ch| matches!(ch, '\r' | '\n' | '\0'))
                || url::Url::parse(value).is_err()
            {
                return Err("invalid history request URL");
            }
            bytes += value.len();
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_state_round_trip_preserves_absent_referrer_and_opaque_initiator() {
        let state = NavigationRequestState {
            initiator_url: Some("about:blank".into()),
            opaque_initiator: true,
            referrer: None,
            referrer_policy: "no-referrer".into(),
        };
        let encoded = serde_json::to_vec(&state).unwrap();
        let decoded: NavigationRequestState = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, state);
        assert!(decoded.retained_bytes().unwrap() >= std::mem::size_of::<NavigationRequestState>());
    }
    #[test]
    fn serialized_request_state_rejects_invalid_and_unbounded_metadata() {
        let mut state = NavigationRequestState {
            initiator_url: Some("https://a.test/".into()),
            opaque_initiator: false,
            referrer: Some("https://a.test/".into()),
            referrer_policy: "strict-origin-when-cross-origin".into(),
        };
        assert!(state.retained_bytes().is_ok());
        for bad in [
            "not a URL".to_owned(),
            "https://a.test/\r\ninjected".into(),
            format!("https://a.test/{}", "a".repeat(64 * 1024)),
        ] {
            state.referrer = Some(bad);
            assert!(state.retained_bytes().is_err());
        }
        state.referrer = None;
        state.referrer_policy = "unknown".into();
        assert!(state.retained_bytes().is_err());
    }
}
