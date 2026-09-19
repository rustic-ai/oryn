//! Explicit immutable web-feature selection for a whole browser context.
//! Profiles contain no URL matching or expected test outcomes.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserMediaElementMode {
    #[default]
    Standard,
    Legacy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFeatureProfile {
    pub usermedia_element: UserMediaElementMode,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_profile_roundtrip_is_independent_of_document_url() {
        for mode in [UserMediaElementMode::Standard, UserMediaElementMode::Legacy] {
            let profile = NativeFeatureProfile {
                usermedia_element: mode,
            };
            assert_eq!(
                serde_json::from_str::<NativeFeatureProfile>(
                    &serde_json::to_string(&profile).unwrap()
                )
                .unwrap(),
                profile
            );
        }
        assert_eq!(
            NativeFeatureProfile::default().usermedia_element,
            UserMediaElementMode::Standard
        );
    }
    #[test]
    fn profile_rejects_implicit_unknown_and_url_selected_modes() {
        for source in [
            "{}",
            r#"{"usermedia_element":true}"#,
            r#"{"usermedia_element":"auto"}"#,
            r#"{"usermedia_element":{"/legacy/":"legacy"}}"#,
            r#"{"usermedia_element":"legacy","url":"/legacy/"}"#,
        ] {
            assert!(serde_json::from_str::<NativeFeatureProfile>(source).is_err());
        }
    }
}
