//! Pointer-free marker payloads. A marker is valid only for the same native
//! document lifetime, semantic target and UTF-16 content; geometry may change.
use super::*;
use sha2::{Digest, Sha256};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Marker {
    pub control: bool,
    pub boundary: Option<oryn_js::accessibility::SourceBoundary>,
    pub boundary_revision: u64,
    pub namespace: [u8; 16],
    pub target: SemanticRef,
    pub realm: u32,
    pub epoch: u64,
    pub offset: u32,
    pub digest: [u8; 32],
}
pub(super) const LENGTH: usize = 136;
fn digest(units: &[u16]) -> [u8; 32] {
    let mut digest = Sha256::new();
    for unit in units {
        digest.update(unit.to_le_bytes());
    }
    digest.finalize().into()
}
impl Marker {
    pub fn new(
        namespace: [u8; 16],
        target: SemanticRef,
        scope: &crate::host::webdriver_script::AutomationRealmSnapshot,
        text: &oryn_js::accessibility::TextSnapshot,
        offset: u32,
    ) -> Option<Self> {
        if text.protected
            || offset as usize > text.value.len()
            || !scope.live
            || scope.document_generation != target.document_generation
        {
            return None;
        }
        Some(Self {
            control: text.control.is_some(),
            boundary: if let Some(container) = text.control {
                let point = oryn_dom::BoundaryPoint { container, offset };
                Some(oryn_js::accessibility::SourceBoundary {
                    start: point,
                    end: point,
                })
            } else if text.boundaries.is_empty() {
                None
            } else {
                Some(*text.boundaries.get(offset as usize)?)
            },
            boundary_revision: text.boundary_revision,
            namespace,
            target,
            realm: scope.realm.0,
            epoch: scope.stream_epoch,
            offset,
            digest: digest(&text.value),
        })
    }
    pub fn valid(
        &self,
        namespace: [u8; 16],
        target: SemanticRef,
        scope: &crate::host::webdriver_script::AutomationRealmSnapshot,
        text: &oryn_js::accessibility::TextSnapshot,
    ) -> bool {
        Self::new(namespace, target, scope, text, self.offset).as_ref() == Some(self)
    }
    pub fn resolve(
        &self,
        namespace: [u8; 16],
        target: SemanticRef,
        scope: &crate::host::webdriver_script::AutomationRealmSnapshot,
        text: &oryn_js::accessibility::TextSnapshot,
        journal: Option<&oryn_dom::boundary_journal::Snapshot>,
    ) -> Option<Self> {
        if self.namespace != namespace
            || self.target.page_id != target.page_id
            || self.target.document_generation != target.document_generation
            || self.realm != scope.realm.0
            || self.epoch != scope.stream_epoch
            || !scope.live
            || text.protected
        {
            return None;
        }
        let Some(boundary) = self.boundary else {
            return self
                .valid(namespace, target, scope, text)
                .then(|| self.clone());
        };
        let journal = journal?;
        if text.boundary_revision != journal.current {
            return None;
        }
        if self.control {
            if text.control != Some(boundary.start.container) || boundary.start != boundary.end {
                return None;
            }
            let point = journal.rebase_control(boundary.start, self.boundary_revision)?;
            return Self::new(namespace, target, scope, text, point.offset);
        }
        if text.control.is_some() {
            return None;
        }
        let start = journal.rebase(boundary.start, self.boundary_revision)?;
        let end = journal.rebase(boundary.end, self.boundary_revision)?;
        let matches = |point: &oryn_js::accessibility::SourceBoundary| {
            point.start == start && point.end == end
        };
        let offset = if self.boundary_revision == text.boundary_revision
            && text
                .boundaries
                .get(self.offset as usize)
                .is_some_and(matches)
        {
            self.offset as usize
        } else {
            text.boundaries.iter().position(matches)?
        };
        Self::new(namespace, target, scope, text, u32::try_from(offset).ok()?)
    }
    pub fn encode(&self) -> [u8; LENGTH] {
        let mut out = [0; LENGTH];
        out[..8].copy_from_slice(b"ORYNAX03");
        let mut at = 8;
        for value in [
            self.target.page_id.0,
            self.target.document_generation,
            self.target.node,
        ] {
            out[at..at + 8].copy_from_slice(&value.to_le_bytes());
            at += 8;
        }
        out[at..at + 4].copy_from_slice(&self.realm.to_le_bytes());
        at += 4;
        out[at..at + 8].copy_from_slice(&self.epoch.to_le_bytes());
        at += 8;
        out[at..at + 4].copy_from_slice(&self.offset.to_le_bytes());
        at += 4;
        out[at..at + 32].copy_from_slice(&self.digest);
        out[80..96].copy_from_slice(&self.namespace);
        out[96..104].copy_from_slice(&self.boundary_revision.to_le_bytes());
        out[105] = u8::from(self.control);
        if let Some(boundary) = self.boundary {
            out[104] = 1;
            for (at, point) in [(112, boundary.start), (124, boundary.end)] {
                out[at..at + 4].copy_from_slice(&point.container.slot.to_le_bytes());
                out[at + 4..at + 8].copy_from_slice(&point.container.generation.to_le_bytes());
                out[at + 8..at + 12].copy_from_slice(&point.offset.to_le_bytes());
            }
        }
        out
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != LENGTH
            || &bytes[..8] != b"ORYNAX03"
            || bytes[104] > 1
            || bytes[105] > 1
            || (bytes[105] == 1 && bytes[104] != 1)
            || bytes[106..112].iter().any(|byte| *byte != 0)
        {
            return None;
        }
        let read = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let point = |at| oryn_dom::BoundaryPoint {
            container: oryn_dom::NodeId {
                slot: u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
                generation: u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()),
            },
            offset: u32::from_le_bytes(bytes[at + 8..at + 12].try_into().unwrap()),
        };
        Some(Self {
            control: bytes[105] == 1,
            boundary: if bytes[104] == 1 {
                Some(oryn_js::accessibility::SourceBoundary {
                    start: point(112),
                    end: point(124),
                })
            } else {
                None
            },
            boundary_revision: read(96),
            namespace: bytes[80..96].try_into().ok()?,
            target: SemanticRef {
                page_id: PageId(read(8)),
                document_generation: read(16),
                node: read(24),
            },
            realm: u32::from_le_bytes(bytes[32..36].try_into().ok()?),
            epoch: read(36),
            offset: u32::from_le_bytes(bytes[44..48].try_into().ok()?),
            digest: bytes[48..80].try_into().ok()?,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marker_roundtrip_rejects_epoch_target_and_content_changes() {
        let target = SemanticRef {
            page_id: PageId(4),
            document_generation: 2,
            node: 9,
        };
        let scope = crate::host::webdriver_script::AutomationRealmSnapshot {
            realm: oryn_js::realm::RealmId(3),
            parent_realm: None,
            frame: None,
            frame_generation: None,
            frame_token: None,
            document_root: oryn_dom::NodeId {
                slot: 1,
                generation: 1,
            },
            document_generation: 2,
            stream_epoch: 7,
            document_authority_epoch: 0,
            url: "https://example.test".into(),
            live: true,
        };
        let mut text = oryn_js::accessibility::TextSnapshot {
            objects: Vec::new(),
            control: None,
            boundary_revision: 0,
            boundaries: Vec::new(),
            protected_caret: None,
            value: vec![97, 0xd800, 0xd83d, 0xde00],
            start: 0,
            end: 0,
            selection_revision: 1,
            protected: false,
            boxes: vec![],
            carets: vec![],
        };
        let marker = Marker::new([7; 16], target, &scope, &text, 2).unwrap();
        assert_eq!(Marker::decode(&marker.encode()), Some(marker.clone()));
        assert!(!marker.valid([8; 16], target, &scope, &text));
        text.selection_revision += 1;
        assert!(marker.valid([7; 16], target, &scope, &text));
        let mut other = scope.clone();
        other.stream_epoch += 1;
        assert!(!marker.valid([7; 16], target, &other, &text));
        text.value[1] = 98;
        assert!(!marker.valid([7; 16], target, &scope, &text));
        text.protected = true;
        assert!(Marker::new([7; 16], target, &scope, &text, 0).is_none());
        assert!(Marker::decode(&[0; LENGTH]).is_none());
        assert!(Marker::decode(&marker.encode()[..LENGTH - 1]).is_none());
    }
    #[test]
    fn control_marker_v3_rebases_value_offsets_and_rejects_unknown_or_reset_identity() {
        let target = SemanticRef {
            page_id: PageId(1),
            document_generation: 2,
            node: 9,
        };
        let control = oryn_dom::NodeId {
            slot: 9,
            generation: 1,
        };
        let scope = crate::host::webdriver_script::AutomationRealmSnapshot {
            realm: oryn_js::realm::RealmId(3),
            parent_realm: None,
            frame: None,
            frame_generation: None,
            frame_token: None,
            document_root: oryn_dom::NodeId {
                slot: 1,
                generation: 1,
            },
            document_generation: 2,
            stream_epoch: 7,
            document_authority_epoch: 0,
            url: "https://example.test".into(),
            live: true,
        };
        let mut text = oryn_js::accessibility::TextSnapshot {
            objects: Vec::new(),
            control: Some(control),
            boundary_revision: 0,
            boundaries: vec![],
            protected_caret: None,
            value: vec![97; 4],
            start: 0,
            end: 0,
            selection_revision: 1,
            protected: false,
            boxes: vec![],
            carets: vec![],
        };
        let marker = Marker::new([1; 16], target, &scope, &text, 2).unwrap();
        assert!(marker.control);
        assert_eq!(Marker::decode(&marker.encode()), Some(marker.clone()));
        let journal = oryn_dom::boundary_journal::Snapshot {
            oldest: 1,
            current: 1,
            complete: true,
            records: vec![oryn_dom::boundary_journal::Record {
                sequence: 1,
                change: oryn_dom::boundary_journal::Change::ControlValueReplace {
                    node: control,
                    offset: 0,
                    removed: 0,
                    inserted: 1,
                },
            }],
        };
        text.boundary_revision = 1;
        text.value.push(97);
        assert_eq!(
            marker
                .resolve([1; 16], target, &scope, &text, Some(&journal))
                .unwrap()
                .offset,
            3
        );
        text.control = None;
        assert!(
            marker
                .resolve([1; 16], target, &scope, &text, Some(&journal))
                .is_none()
        );
        let mut legacy = marker.encode();
        legacy[..8].copy_from_slice(b"ORYNAX02");
        assert!(Marker::decode(&legacy).is_none());
    }
    #[test]
    fn live_marker_rebases_exact_native_boundary_and_rejects_missing_history() {
        let target = SemanticRef {
            page_id: PageId(1),
            document_generation: 2,
            node: 9,
        };
        let scope = crate::host::webdriver_script::AutomationRealmSnapshot {
            realm: oryn_js::realm::RealmId(3),
            parent_realm: None,
            frame: None,
            frame_generation: None,
            frame_token: None,
            document_root: oryn_dom::NodeId {
                slot: 1,
                generation: 1,
            },
            document_generation: 2,
            stream_epoch: 7,
            document_authority_epoch: 0,
            url: "https://example.test".into(),
            live: true,
        };
        let source = oryn_dom::NodeId {
            slot: 8,
            generation: 1,
        };
        let points = |length| {
            (0..=length)
                .map(|offset| {
                    let point = oryn_dom::BoundaryPoint {
                        container: source,
                        offset,
                    };
                    oryn_js::accessibility::SourceBoundary {
                        start: point,
                        end: point,
                    }
                })
                .collect()
        };
        let mut text = oryn_js::accessibility::TextSnapshot {
            objects: Vec::new(),
            control: None,
            boundary_revision: 0,
            boundaries: points(4),
            protected_caret: None,
            value: vec![97; 4],
            start: 0,
            end: 0,
            selection_revision: 1,
            protected: false,
            boxes: vec![],
            carets: vec![],
        };
        let marker = Marker::new([1; 16], target, &scope, &text, 2).unwrap();
        assert_eq!(Marker::decode(&marker.encode()), Some(marker.clone()));
        let journal = oryn_dom::boundary_journal::Snapshot {
            oldest: 1,
            current: 1,
            complete: true,
            records: vec![oryn_dom::boundary_journal::Record {
                sequence: 1,
                change: oryn_dom::boundary_journal::Change::Replace {
                    node: source,
                    offset: 1,
                    removed: 0,
                    inserted: 2,
                },
            }],
        };
        text.boundary_revision = 1;
        text.value = vec![97; 6];
        text.boundaries = points(6);
        assert_eq!(
            marker
                .resolve([1; 16], target, &scope, &text, Some(&journal))
                .unwrap()
                .offset,
            4
        );
        let missing = oryn_dom::boundary_journal::Snapshot {
            oldest: 2,
            current: 1,
            complete: true,
            records: vec![],
        };
        assert!(
            marker
                .resolve([1; 16], target, &scope, &text, Some(&missing))
                .is_none()
        );
        text.protected = true;
        assert!(
            marker
                .resolve([1; 16], target, &scope, &text, Some(&journal))
                .is_none()
        );
    }
}
