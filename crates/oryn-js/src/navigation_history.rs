//! Serializable navigation-history data shared by saved documents and V8 hosts.
//! This module owns no engine handles and is available with default features.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryScrollMode {
    #[default]
    Auto,
    Manual,
}
impl HistoryScrollMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
        }
    }
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct HistoryScrollState {
    pub mode: HistoryScrollMode,
    pub position: Option<(f64, f64)>,
}
impl HistoryScrollState {
    pub fn valid(&self) -> bool {
        self.position
            .is_none_or(|(x, y)| x.is_finite() && y.is_finite())
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct NavigationHistorySnapshot {
    #[serde(default)]
    pub scroll: HistoryScrollState,
    pub key: String,
    pub id: String,
    pub url: String,
    pub document_id: String,
    pub state: Option<crate::message_wire::WireValue>,
    #[serde(default)]
    pub history_state: Option<crate::message_wire::WireValue>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct NavigationHistoryUpdate {
    pub entries: Vec<NavigationHistorySnapshot>,
    pub current: usize,
    pub navigation_type: String,
    #[serde(default)]
    pub truncates_forward: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(bytes: Vec<u8>) -> crate::message_wire::WireValue {
        crate::message_wire::WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes,
            hosts: Vec::new(),
            buffers: vec![vec![0, 255, 128]],
            ports: Vec::new(),
        }
    }

    fn history() -> NavigationHistoryUpdate {
        NavigationHistoryUpdate {
            entries: vec![NavigationHistorySnapshot {
                scroll: HistoryScrollState {
                    mode: HistoryScrollMode::Manual,
                    position: Some((12.5, -3.25)),
                },
                key: "entry-key".into(),
                id: "entry-id".into(),
                url: "https://history.test/page#saved".into(),
                document_id: "document-id".into(),
                state: Some(wire(vec![0, 255, 1])),
                history_state: Some(wire(vec![128, 0, 2])),
            }],
            current: 0,
            navigation_type: "traverse".into(),
            truncates_forward: true,
        }
    }

    #[test]
    fn portable_history_wire_preserves_identity_scroll_and_independent_states() {
        let source = history();
        let expected = serde_json::json!({
            "entries": [{
                "scroll": {"mode": "manual", "position": [12.5, -3.25]},
                "key": "entry-key", "id": "entry-id",
                "url": "https://history.test/page#saved", "document_id": "document-id",
                "state": serde_json::to_value(wire(vec![0, 255, 1])).unwrap(),
                "history_state": serde_json::to_value(wire(vec![128, 0, 2])).unwrap(),
            }],
            "current": 0, "navigation_type": "traverse", "truncates_forward": true,
        });
        assert_eq!(serde_json::to_value(&source).unwrap(), expected);
        let restored: NavigationHistoryUpdate =
            serde_json::from_slice(&serde_json::to_vec(&source).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), expected);
        let entry = &restored.entries[0];
        assert_eq!(entry.state.as_ref().unwrap().bytes, [0, 255, 1]);
        assert_eq!(entry.history_state.as_ref().unwrap().bytes, [128, 0, 2]);
        assert_eq!(entry.state.as_ref().unwrap().buffers, [vec![0, 255, 128]]);
        assert_eq!(entry.scroll.mode.as_str(), "manual");
        assert!(entry.scroll.valid());
    }

    #[test]
    fn portable_history_legacy_defaults_preserve_scroll_validation() {
        let legacy = serde_json::json!({
            "entries": [{"key": "key", "id": "id", "url": "https://history.test/", "document_id": "document", "state": null}],
            "current": 0, "navigation_type": "traverse",
        });
        let restored: NavigationHistoryUpdate = serde_json::from_value(legacy.clone()).unwrap();
        assert!(!restored.truncates_forward);
        assert_eq!(restored.entries[0].scroll.mode, HistoryScrollMode::Auto);
        assert_eq!(restored.entries[0].scroll.mode.as_str(), "auto");
        assert_eq!(restored.entries[0].scroll.position, None);
        assert!(restored.entries[0].history_state.is_none());
        assert!(restored.entries[0].scroll.valid());
        for position in [
            (f64::INFINITY, 0.0),
            (0.0, f64::NEG_INFINITY),
            (f64::NAN, 0.0),
            (0.0, f64::NAN),
        ] {
            assert!(
                !HistoryScrollState {
                    mode: HistoryScrollMode::Auto,
                    position: Some(position)
                }
                .valid()
            );
        }
        let mut invalid = legacy;
        invalid["entries"][0]["scroll"] = serde_json::json!({"mode": "MANUAL", "position": null});
        assert!(serde_json::from_value::<NavigationHistoryUpdate>(invalid).is_err());
    }

    #[cfg(feature = "v8-host")]
    #[test]
    fn v8_history_exports_share_the_portable_type_identity() {
        use std::any::TypeId;
        assert_eq!(
            TypeId::of::<HistoryScrollMode>(),
            TypeId::of::<crate::navigation_api::HistoryScrollMode>()
        );
        assert_eq!(
            TypeId::of::<HistoryScrollState>(),
            TypeId::of::<crate::navigation_api::HistoryScrollState>()
        );
        assert_eq!(
            TypeId::of::<NavigationHistorySnapshot>(),
            TypeId::of::<crate::navigation_api::NavigationHistorySnapshot>()
        );
        assert_eq!(
            TypeId::of::<NavigationHistoryUpdate>(),
            TypeId::of::<crate::navigation_api::NavigationHistoryUpdate>()
        );
        let native: crate::navigation_api::NavigationHistoryUpdate = history();
        let portable: NavigationHistoryUpdate = native;
        assert_eq!(portable.entries[0].document_id, "document-id");
        assert_eq!(
            portable.entries[0].state.as_ref().unwrap().bytes,
            [0, 255, 1]
        );
    }
}
