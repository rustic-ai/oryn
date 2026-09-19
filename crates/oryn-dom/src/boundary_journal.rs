//! Numeric native boundary transformations for snapshot consumers. No text,
//! author objects or native pointer capabilities cross this journal.
use crate::{BoundaryPoint, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Change {
    Noop,
    Insert {
        parent: NodeId,
        index: u32,
        count: u32,
    },
    Remove {
        parent: NodeId,
        index: u32,
        nodes: Vec<NodeId>,
    },
    Replace {
        node: NodeId,
        offset: u32,
        removed: u32,
        inserted: u32,
    },
    Normalize {
        from: NodeId,
        to: NodeId,
        parent: NodeId,
        index: u32,
        target_offset: u32,
    },
    Split {
        node: NodeId,
        created: NodeId,
        offset: u32,
        parent: Option<(NodeId, u32)>,
    },
    ControlValueReplace {
        node: NodeId,
        offset: u32,
        removed: u32,
        inserted: u32,
    },
    ControlValueReset {
        node: NodeId,
    },
}
impl Change {
    pub fn apply(&self, point: &mut BoundaryPoint) {
        match self {
            Self::Noop | Self::ControlValueReplace { .. } | Self::ControlValueReset { .. } => {}
            Self::Insert {
                parent,
                index,
                count,
            } => {
                if point.container == *parent && point.offset > *index {
                    point.offset = point.offset.saturating_add(*count);
                }
            }
            Self::Remove {
                parent,
                index,
                nodes,
            } => {
                if nodes.binary_search(&point.container).is_ok() {
                    *point = BoundaryPoint {
                        container: *parent,
                        offset: *index,
                    };
                } else if point.container == *parent && point.offset > *index {
                    point.offset -= 1;
                }
            }
            Self::Replace {
                node,
                offset,
                removed,
                inserted,
            } => {
                if point.container == *node && point.offset > *offset {
                    point.offset = if point.offset <= offset.saturating_add(*removed) {
                        *offset
                    } else {
                        point
                            .offset
                            .saturating_sub(*removed)
                            .saturating_add(*inserted)
                    };
                }
            }
            Self::Normalize {
                from,
                to,
                parent,
                index,
                target_offset,
            } => {
                if point.container == *from {
                    point.container = *to;
                    point.offset = point.offset.saturating_add(*target_offset);
                } else if point.container == *parent && point.offset == *index {
                    point.container = *to;
                    point.offset = *target_offset;
                }
            }
            Self::Split {
                node,
                created,
                offset,
                parent,
            } => {
                if point.container == *node && point.offset > *offset {
                    if parent.is_some() {
                        point.container = *created;
                        point.offset -= *offset;
                    } else {
                        point.offset = *offset;
                    }
                } else if parent.is_some_and(|(parent, index)| {
                    point.container == parent && point.offset >= index
                }) {
                    point.offset = point.offset.saturating_add(1);
                }
            }
        }
    }
    fn weight(&self) -> usize {
        match self {
            Self::Remove { nodes, .. } => 1 + nodes.len(),
            _ => 1,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub sequence: u64,
    pub change: Change,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub oldest: u64,
    pub current: u64,
    pub complete: bool,
    pub records: Vec<Record>,
}
impl Snapshot {
    pub fn rebase_control(&self, mut point: BoundaryPoint, revision: u64) -> Option<BoundaryPoint> {
        if !self.complete || revision > self.current || revision < self.oldest.saturating_sub(1) {
            return None;
        }
        let mut previous = self.oldest.saturating_sub(1);
        for record in &self.records {
            if record.sequence != previous.checked_add(1)? {
                return None;
            }
            previous = record.sequence;
            if record.sequence <= revision {
                continue;
            }
            match &record.change {
                Change::ControlValueReplace {
                    node,
                    offset,
                    removed,
                    inserted,
                } if *node == point.container => {
                    if point.offset > *offset {
                        point.offset = if point.offset <= offset.checked_add(*removed)? {
                            *offset
                        } else {
                            point.offset.checked_sub(*removed)?.checked_add(*inserted)?
                        };
                    }
                }
                Change::ControlValueReset { node } if *node == point.container => return None,
                Change::Remove { nodes, .. } if nodes.binary_search(&point.container).is_ok() => {
                    return None;
                }
                _ => {}
            }
        }
        (previous == self.current).then_some(point)
    }

    pub fn rebase(&self, mut point: BoundaryPoint, revision: u64) -> Option<BoundaryPoint> {
        if !self.complete || revision > self.current || revision < self.oldest.saturating_sub(1) {
            return None;
        }
        let mut previous = self.oldest.saturating_sub(1);
        for record in &self.records {
            if record.sequence != previous.checked_add(1)? {
                return None;
            }
            previous = record.sequence;
            if record.sequence > revision {
                record.change.apply(&mut point);
            }
        }
        if previous != self.current {
            return None;
        }
        Some(point)
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Journal {
    records: VecDeque<Record>,
    current: u64,
    weight: usize,
    complete: bool,
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            records: VecDeque::new(),
            current: 0,
            weight: 0,
            complete: true,
        }
    }
}
impl Journal {
    pub fn revision(&self) -> u64 {
        self.current
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            oldest: self
                .records
                .front()
                .map_or(self.current.saturating_add(1), |record| record.sequence),
            current: self.current,
            complete: self.complete,
            records: self.records.iter().cloned().collect(),
        }
    }
    pub fn push(&mut self, change: Change) {
        let Some(next) = self.current.checked_add(1) else {
            self.complete = false;
            return;
        };
        self.current = next;
        self.weight = self.weight.saturating_add(change.weight());
        self.records.push_back(Record {
            sequence: next,
            change,
        });
        while self.records.len() > 16_384 || self.weight > 1_048_576 {
            let Some(record) = self.records.pop_front() else {
                break;
            };
            self.weight = self.weight.saturating_sub(record.change.weight());
        }
    }
    /// A native split has no author checkpoint between its replacement/insertion.
    /// Replace only those exact primitive adjustments, preserving unrelated work.
    pub fn finish_split(
        &mut self,
        since: u64,
        node: NodeId,
        created: NodeId,
        offset: u32,
        parent: Option<(NodeId, u32)>,
    ) {
        let mut replacement = false;
        let mut insertion = parent.is_none();
        for record in self
            .records
            .iter_mut()
            .filter(|record| record.sequence > since)
        {
            let matches = match &record.change {
                Change::Replace {
                    node: target,
                    offset: start,
                    inserted: 0,
                    ..
                } if *target == node && *start == offset && !replacement => {
                    replacement = true;
                    true
                }
                Change::Insert {
                    parent: owner,
                    index,
                    count: 1,
                } if parent == Some((*owner, *index)) && !insertion => {
                    insertion = true;
                    true
                }
                _ => false,
            };
            if matches {
                record.change = Change::Noop;
            }
        }
        if !replacement || !insertion {
            self.complete = false;
        }
        self.push(Change::Split {
            node,
            created,
            offset,
            parent,
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn node(slot: u32) -> NodeId {
        NodeId {
            slot,
            generation: 1,
        }
    }
    #[test]
    fn exact_replace_rebase_does_not_guess_from_repeated_text() {
        let mut journal = Journal::default();
        journal.push(Change::Replace {
            node: node(1),
            offset: 2,
            removed: 0,
            inserted: 3,
        });
        let snapshot = journal.snapshot();
        assert_eq!(
            snapshot
                .rebase(
                    BoundaryPoint {
                        container: node(1),
                        offset: 4
                    },
                    0
                )
                .unwrap()
                .offset,
            7
        );
        assert_eq!(
            snapshot
                .rebase(
                    BoundaryPoint {
                        container: node(1),
                        offset: 2
                    },
                    0
                )
                .unwrap()
                .offset,
            2
        );
    }
    #[test]
    fn split_replaces_primitive_clamping_and_preserves_original_offset() {
        let mut journal = Journal::default();
        journal.push(Change::Replace {
            node: node(1),
            offset: 2,
            removed: 4,
            inserted: 0,
        });
        journal.push(Change::Insert {
            parent: node(3),
            index: 1,
            count: 1,
        });
        journal.finish_split(0, node(1), node(2), 2, Some((node(3), 1)));
        let snapshot = journal.snapshot();
        assert_eq!(
            snapshot.rebase(
                BoundaryPoint {
                    container: node(1),
                    offset: 5
                },
                0
            ),
            Some(BoundaryPoint {
                container: node(2),
                offset: 3
            })
        );
        assert_eq!(
            snapshot
                .rebase(
                    BoundaryPoint {
                        container: node(3),
                        offset: 1
                    },
                    0
                )
                .unwrap()
                .offset,
            2
        );
    }
    #[test]
    fn missing_history_and_retired_generations_never_silently_rebase() {
        let mut journal = Journal::default();
        for _ in 0..16_385 {
            journal.push(Change::Insert {
                parent: node(1),
                index: 0,
                count: 1,
            });
        }
        assert!(
            journal
                .snapshot()
                .rebase(
                    BoundaryPoint {
                        container: node(1),
                        offset: 1
                    },
                    0
                )
                .is_none()
        );
        let point = BoundaryPoint {
            container: NodeId {
                slot: 1,
                generation: 2,
            },
            offset: 1,
        };
        assert_eq!(journal.snapshot().rebase(point, 1), Some(point));
    }
}
