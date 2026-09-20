//! Parent custody for accepted Window events. Admission and publication run
//! under one journal lock; producers admit before committing author effects.
use crate::{
    PageId,
    context_service::{ContextEvent, ContextEventRecord},
};
use oryn_js::worker_wire::Creator;
use std::collections::VecDeque;

const MAX_EVENTS: usize = 4096;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_EVENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_RECIPIENTS: usize = 65_536;
const MAX_CURSORS: usize = 8192;
const DIAGNOSTIC_EVENTS: usize = 256;
const DIAGNOSTIC_BYTES: usize = 1024 * 1024;
const MAX_ID: u64 = (1u64 << 53) - 1;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recipient {
    pub page: PageId,
    pub creator: Creator,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Channel {
    Storage,
    Csp,
    Compatibility,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Evidence {
    pub diagnostics_pruned: u64,
    pub diagnostic_bytes_pruned: u64,
    pub diagnostics_rejected: u64,
    pub retired_deliveries: u64,
    pub undeliverable_events: u64,
    pub last_published_id: u64,
}
struct Entry {
    record: ContextEventRecord,
    pending: Vec<Recipient>,
    bytes: usize,
    diagnostic: bool,
}
struct Cursor {
    recipient: Recipient,
    channel: Channel,
    delivered: u64,
    acknowledged: u64,
}
pub(crate) struct Prepared {
    entries: Vec<Entry>,
    bytes: usize,
    recipients: usize,
}
pub(crate) struct Journal {
    entries: VecDeque<Entry>,
    cursors: Vec<Cursor>,
    next: u64,
    bytes: usize,
    recipients: usize,
    evidence: Evidence,
}
impl Default for Journal {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            cursors: Vec::new(),
            next: 1,
            bytes: 0,
            recipients: 0,
            evidence: Evidence::default(),
        }
    }
}
fn diagnostic(event: &ContextEvent) -> bool {
    matches!(
        event,
        ContextEvent::CspReport { .. } | ContextEvent::PageLifecycle { .. }
    )
}
fn channel_matches(event: &ContextEvent, channel: Channel) -> bool {
    match channel {
        Channel::Storage => matches!(event, ContextEvent::Storage { .. }),
        Channel::Csp => matches!(event, ContextEvent::CspViolation { .. }),
        Channel::Compatibility => !matches!(event, ContextEvent::PageLifecycle { .. }),
    }
}
fn recipients(event: &ContextEvent) -> Vec<Recipient> {
    match event {
        ContextEvent::Storage { targets, .. } => {
            let mut recipients = Vec::new();
            for (page, creator) in targets {
                let recipient = Recipient {
                    page: *page,
                    creator: creator.clone(),
                };
                if !recipients.contains(&recipient) {
                    recipients.push(recipient);
                }
            }
            recipients
        }
        ContextEvent::CspViolation {
            target_page_id,
            target_creator: Some(creator),
            ..
        } => vec![Recipient {
            page: *target_page_id,
            creator: creator.clone(),
        }],
        _ => Vec::new(),
    }
}
struct Size {
    bytes: usize,
}
impl std::io::Write for Size {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_EVENT_BYTES)
            .ok_or_else(|| std::io::Error::other("context event exceeds delivery frame budget"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn size(event: &ContextEvent) -> Result<usize, String> {
    let mut size = Size { bytes: 64 };
    serde_json::to_writer(&mut size, event).map_err(|error| error.to_string())?;
    Ok(size.bytes)
}
impl Journal {
    #[cfg(test)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = &ContextEventRecord> {
        self.entries.iter().map(|entry| &entry.record)
    }
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    #[cfg(test)]
    pub(crate) fn evidence(&self) -> Evidence {
        self.evidence.clone()
    }
    fn remove(&mut self, index: usize) {
        if let Some(entry) = self.entries.remove(index) {
            self.bytes -= entry.bytes;
            self.recipients -= entry.pending.len();
            if entry.diagnostic {
                self.evidence.diagnostics_pruned =
                    self.evidence.diagnostics_pruned.saturating_add(1);
                self.evidence.diagnostic_bytes_pruned = self
                    .evidence
                    .diagnostic_bytes_pruned
                    .saturating_add(entry.bytes as u64);
            }
        }
    }
    /// This method must remain under the same lock until commit or cancellation.
    /// The prepared payload owns the exact event data admitted before mutation.
    pub(crate) fn prepare(&mut self, events: Vec<ContextEvent>) -> Result<Prepared, String> {
        if events.len() > MAX_EVENTS {
            return Err("context event batch capacity exhausted".into());
        }
        let mut entries = Vec::new();
        entries
            .try_reserve(events.len())
            .map_err(|_| "context event admission allocation")?;
        let mut bytes = 0usize;
        let mut recipient_count = 0usize;
        for event in events {
            if diagnostic(&event) {
                return Err("diagnostic requires bounded evidence publication".into());
            }
            if matches!(&event,ContextEvent::Storage{targets,..}if targets.len()>MAX_RECIPIENTS) {
                return Err("context event recipient capacity exhausted".into());
            }
            let pending = recipients(&event);
            let cost = size(&event)?;
            if matches!(event, ContextEvent::Message { .. }) {
                return Err("legacy messages require native MessageHub recipient custody".into());
            }
            bytes = bytes
                .checked_add(cost)
                .ok_or("context event byte overflow")?;
            recipient_count = recipient_count
                .checked_add(pending.len())
                .ok_or("context event recipient overflow")?;
            entries.push(Entry {
                record: ContextEventRecord { event_id: 0, event },
                pending,
                bytes: cost,
                diagnostic: false,
            });
        }
        if self
            .next
            .checked_add(entries.len() as u64)
            .is_none_or(|next| next > MAX_ID + 1)
        {
            return Err("context event identity exhausted".into());
        }
        // Diagnostic history is expendable only with explicit pruning evidence.
        while self.entries.len() + entries.len() > MAX_EVENTS
            || self
                .bytes
                .checked_add(bytes)
                .is_none_or(|bytes| bytes > MAX_BYTES)
        {
            let Some(index) = self.entries.iter().position(|entry| entry.diagnostic) else {
                return Err("context event custody capacity exhausted".into());
            };
            self.remove(index);
        }
        if self
            .recipients
            .checked_add(recipient_count)
            .is_none_or(|count| count > MAX_RECIPIENTS)
        {
            return Err("context event recipient capacity exhausted".into());
        }
        self.entries
            .try_reserve(entries.len())
            .map_err(|_| "context event publication allocation")?;
        Ok(Prepared {
            entries,
            bytes,
            recipients: recipient_count,
        })
    }
    pub(crate) fn commit(
        &mut self,
        mut prepared: Prepared,
        mut retained: impl FnMut(&Recipient) -> bool,
    ) {
        // No allocation, admission failure, or identity arithmetic remains
        // after the producer's effect commits. Publication order owns IDs.
        let _ = (prepared.bytes, prepared.recipients);
        for mut entry in prepared.entries.drain(..) {
            let before = entry.pending.len();
            entry.pending.retain(|recipient| retained(recipient));
            self.evidence.retired_deliveries = self
                .evidence
                .retired_deliveries
                .saturating_add((before - entry.pending.len()) as u64);
            if entry.pending.is_empty() {
                self.evidence.undeliverable_events =
                    self.evidence.undeliverable_events.saturating_add(1);
                continue;
            }
            entry.record.event_id = self.next;
            self.next += 1;
            self.evidence.last_published_id = entry.record.event_id;
            self.bytes += entry.bytes;
            self.recipients += entry.pending.len();
            self.entries.push_back(entry);
        }
    }
    pub(crate) fn reject_diagnostic(&mut self) {
        self.evidence.diagnostics_rejected = self.evidence.diagnostics_rejected.saturating_add(1);
    }
    pub(crate) fn record_diagnostic(&mut self, event: ContextEvent) {
        if !diagnostic(&event) {
            self.evidence.diagnostics_rejected =
                self.evidence.diagnostics_rejected.saturating_add(1);
            return;
        }
        let Ok(bytes) = size(&event) else {
            self.evidence.diagnostics_rejected =
                self.evidence.diagnostics_rejected.saturating_add(1);
            return;
        };
        if bytes > DIAGNOSTIC_BYTES || self.next > MAX_ID {
            self.evidence.diagnostics_rejected =
                self.evidence.diagnostics_rejected.saturating_add(1);
            return;
        }
        loop {
            let count = self.entries.iter().filter(|entry| entry.diagnostic).count();
            let used = self
                .entries
                .iter()
                .filter(|entry| entry.diagnostic)
                .map(|entry| entry.bytes)
                .sum::<usize>();
            if count < DIAGNOSTIC_EVENTS
                && used + bytes <= DIAGNOSTIC_BYTES
                && self.entries.len() < MAX_EVENTS
                && self.bytes + bytes <= MAX_BYTES
            {
                break;
            }
            let Some(index) = self.entries.iter().position(|entry| entry.diagnostic) else {
                self.evidence.diagnostics_rejected =
                    self.evidence.diagnostics_rejected.saturating_add(1);
                return;
            };
            self.remove(index);
        }
        if self.entries.try_reserve(1).is_err() {
            self.evidence.diagnostics_rejected =
                self.evidence.diagnostics_rejected.saturating_add(1);
            return;
        }
        let event_id = self.next;
        self.next += 1;
        self.evidence.last_published_id = event_id;
        self.bytes += bytes;
        self.entries.push_back(Entry {
            record: ContextEventRecord { event_id, event },
            pending: Vec::new(),
            bytes,
            diagnostic: true,
        });
    }
    pub(crate) fn prune(&mut self, mut retained: impl FnMut(&Recipient) -> bool) {
        for entry in &mut self.entries {
            let before = entry.pending.len();
            entry.pending.retain(|recipient| retained(recipient));
            let retired = before - entry.pending.len();
            self.recipients -= retired;
            self.evidence.retired_deliveries = self
                .evidence
                .retired_deliveries
                .saturating_add(retired as u64);
        }
        let mut index = 0;
        while index < self.entries.len() {
            if !self.entries[index].diagnostic && self.entries[index].pending.is_empty() {
                self.remove(index);
            } else {
                index += 1;
            }
        }
        self.cursors.retain(|cursor| retained(&cursor.recipient));
    }
    pub(crate) fn retire_page(&mut self, page: PageId) {
        self.prune(|recipient| recipient.page != page);
        let mut index = 0;
        while index < self.entries.len() {
            if matches!(&self.entries[index].record.event,ContextEvent::CspReport{target_page_id,..}if *target_page_id==page)
            {
                self.remove(index);
            } else {
                index += 1;
            }
        }
    }
    pub(crate) fn poll(
        &mut self,
        recipient: Recipient,
        channel: Channel,
        after: u64,
        max: usize,
        mut allowed: impl FnMut(&ContextEvent) -> bool,
    ) -> Result<Vec<ContextEventRecord>, String> {
        let cursor = if let Some(index) = self
            .cursors
            .iter()
            .position(|cursor| cursor.recipient == recipient && cursor.channel == channel)
        {
            index
        } else {
            if after != 0 {
                return Err("context event acknowledgement has no delivery receipt".into());
            }
            if self.cursors.len() >= MAX_CURSORS {
                return Err("context event cursor capacity exhausted".into());
            }
            self.cursors
                .try_reserve(1)
                .map_err(|_| "context event cursor allocation")?;
            self.cursors.push(Cursor {
                recipient: recipient.clone(),
                channel,
                delivered: 0,
                acknowledged: 0,
            });
            self.cursors.len() - 1
        };
        if after > self.cursors[cursor].delivered {
            return Err("context event acknowledgement exceeds delivered receipt".into());
        }
        let after = after.max(self.cursors[cursor].acknowledged);
        self.cursors[cursor].acknowledged = after;
        // The legacy poll_events shim inspects storage and ignores CSP DOM
        // dispatch. Its inspection cursor must never acknowledge native tasks.
        for entry in &mut self.entries {
            if channel != Channel::Compatibility
                && entry.record.event_id <= after
                && channel_matches(&entry.record.event, channel)
            {
                let before = entry.pending.len();
                entry.pending.retain(|target| target != &recipient);
                self.recipients -= before - entry.pending.len();
            }
        }
        let mut index = 0;
        while index < self.entries.len() {
            if !self.entries[index].diagnostic && self.entries[index].pending.is_empty() {
                self.remove(index);
            } else {
                index += 1;
            }
        }
        let mut events = Vec::new();
        let mut bytes = 0usize;
        for entry in &self.entries {
            if entry.record.event_id <= after
                || !channel_matches(&entry.record.event, channel)
                || !allowed(&entry.record.event)
            {
                continue;
            }
            if !entry.diagnostic && !entry.pending.contains(&recipient) {
                continue;
            }
            if events.len() >= max.min(128) || bytes + entry.bytes > MAX_EVENT_BYTES {
                break;
            }
            bytes += entry.bytes;
            events.push(entry.record.clone());
        }
        if let Some(last) = events.last() {
            self.cursors[cursor].delivered = self.cursors[cursor].delivered.max(last.event_id);
        }
        Ok(events)
    }
}
impl std::ops::Index<usize> for Journal {
    type Output = ContextEventRecord;
    fn index(&self, index: usize) -> &Self::Output {
        &self.entries[index].record
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn recipient(page: u64, generation: u64) -> Recipient {
        Recipient {
            page: PageId(page),
            creator: Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: generation,
                stream_epoch: 0,
            },
        }
    }
    fn storage(targets: &[Recipient], value: &str) -> ContextEvent {
        ContextEvent::Storage {
            source_page_id: PageId(99),
            source_creator: recipient(99, 1).creator,
            source_url: "https://example.test/source".into(),
            targets: targets
                .iter()
                .map(|target| (target.page, target.creator.clone()))
                .collect(),
            change: crate::storage::StorageChange {
                area: crate::storage::StorageArea::Local,
                scope: crate::storage::StorageScope {
                    origin: "https://example.test".into(),
                    top_level_page_id: PageId(99),
                },
                key: Some("key".into()),
                old_value: None,
                new_value: Some(value.into()),
            },
        }
    }
    fn publish(journal: &mut Journal, event: ContextEvent) {
        let prepared = journal.prepare(vec![event]).unwrap();
        journal.commit(prepared, |_| true);
    }
    #[test]
    fn fanout_ack_reclaims_only_the_acknowledging_recipient() {
        let mut journal = Journal::default();
        let a = recipient(1, 1);
        let b = recipient(2, 1);
        publish(&mut journal, storage(&[a.clone(), b.clone()], "first"));
        let first = journal
            .poll(a.clone(), Channel::Storage, 0, 32, |_| true)
            .unwrap();
        let id = first[0].event_id;
        assert_eq!(
            journal
                .poll(a.clone(), Channel::Storage, 0, 32, |_| true)
                .unwrap()[0]
                .event_id,
            id,
            "lost response retains event"
        );
        assert!(
            journal
                .poll(a, Channel::Storage, id, 32, |_| true)
                .unwrap()
                .is_empty()
        );
        assert_eq!(journal.len(), 1);
        assert_eq!(journal.recipients, 1);
        assert_eq!(
            journal
                .poll(b.clone(), Channel::Storage, 0, 32, |_| true)
                .unwrap()[0]
                .event_id,
            id
        );
        journal.poll(b, Channel::Storage, id, 32, |_| true).unwrap();
        assert!(journal.is_empty());
        assert_eq!(journal.bytes, 0);
    }
    #[test]
    fn acknowledgement_cannot_cross_channel_or_unreceived_cursor() {
        let mut journal = Journal::default();
        let a = recipient(1, 1);
        publish(&mut journal, storage(std::slice::from_ref(&a), "value"));
        assert!(
            journal
                .poll(a.clone(), Channel::Storage, 100, 32, |_| true)
                .is_err()
        );
        let id = journal
            .poll(a.clone(), Channel::Storage, 0, 32, |_| true)
            .unwrap()[0]
            .event_id;
        assert!(
            journal
                .poll(a.clone(), Channel::Csp, id, 32, |_| true)
                .is_err()
        );
        assert_eq!(journal.recipients, 1);
        journal.poll(a, Channel::Storage, id, 32, |_| true).unwrap();
        assert!(journal.is_empty());
    }
    #[test]
    fn compatibility_inspection_cannot_consume_native_storage_or_csp_ack() {
        let mut journal = Journal::default();
        let a = recipient(1, 1);
        publish(&mut journal, storage(std::slice::from_ref(&a), "native"));
        let mut policy = crate::csp::CspPolicySet::new("https://example.test/");
        policy.install(
            "script-src 'none'",
            crate::csp::CspDisposition::Enforce,
            "header",
        );
        let violation = policy
            .authorize(&crate::csp::CspRequest {
                destination: crate::csp::CspDestination::Script,
                url: "https://example.test/code.js".into(),
                nonce: None,
                inline_source: None,
                parser_inserted: false,
                initiated_by_trusted_script: false,
            })
            .violations
            .remove(0);
        publish(
            &mut journal,
            ContextEvent::CspViolation {
                target_page_id: a.page,
                target_creator: Some(a.creator.clone()),
                violation,
            },
        );
        let inspected = journal
            .poll(a.clone(), Channel::Compatibility, 0, 32, |_| true)
            .unwrap();
        assert_eq!(inspected.len(), 2);
        let storage_id = inspected[0].event_id;
        let csp_id = inspected[1].event_id;
        assert!(
            journal
                .poll(a.clone(), Channel::Compatibility, csp_id, 32, |_| true)
                .unwrap()
                .is_empty()
        );
        assert_eq!(journal.recipients, 2);
        assert_eq!(
            journal
                .poll(a.clone(), Channel::Csp, 0, 32, |_| true)
                .unwrap()[0]
                .event_id,
            csp_id
        );
        journal
            .poll(a.clone(), Channel::Csp, csp_id, 32, |_| true)
            .unwrap();
        assert_eq!(journal.recipients, 1);
        assert_eq!(
            journal
                .poll(a.clone(), Channel::Storage, 0, 32, |_| true)
                .unwrap()[0]
                .event_id,
            storage_id
        );
        journal
            .poll(a, Channel::Storage, storage_id, 32, |_| true)
            .unwrap();
        assert!(journal.is_empty());
    }
    #[test]
    fn saturation_rejects_before_publication_and_ack_restores_capacity() {
        let mut journal = Journal::default();
        let a = recipient(1, 1);
        for _ in 0..MAX_EVENTS {
            publish(&mut journal, storage(std::slice::from_ref(&a), "v"));
        }
        let next = journal.next;
        assert!(
            journal
                .prepare(vec![storage(std::slice::from_ref(&a), "rejected")])
                .is_err()
        );
        assert_eq!(journal.next, next);
        assert_eq!(journal.len(), MAX_EVENTS);
        let first = journal
            .poll(a.clone(), Channel::Storage, 0, 1, |_| true)
            .unwrap()[0]
            .event_id;
        journal
            .poll(a.clone(), Channel::Storage, first, 0, |_| true)
            .unwrap();
        publish(&mut journal, storage(&[a], "accepted"));
        assert_eq!(journal.len(), MAX_EVENTS);
    }
    #[test]
    fn identity_exhaustion_and_cancelled_admission_never_wrap() {
        let mut journal = Journal::default();
        let a = recipient(1, 1);
        journal.next = MAX_ID;
        let cancelled = journal
            .prepare(vec![storage(std::slice::from_ref(&a), "cancel")])
            .unwrap();
        drop(cancelled);
        assert_eq!(journal.next, MAX_ID);
        assert!(journal.is_empty());
        publish(&mut journal, storage(std::slice::from_ref(&a), "last"));
        assert_eq!(journal[0].event_id, MAX_ID);
        assert!(journal.prepare(vec![storage(&[a], "overflow")]).is_err());
        assert_eq!(journal.next, MAX_ID + 1);
    }
    #[test]
    fn late_commit_and_retirement_do_not_address_a_successor() {
        let mut journal = Journal::default();
        let old = recipient(1, 1);
        let successor = recipient(1, 2);
        let prepared = journal
            .prepare(vec![storage(std::slice::from_ref(&old), "late")])
            .unwrap();
        journal.commit(prepared, |recipient| recipient == &successor);
        assert!(journal.is_empty());
        assert_eq!(journal.evidence.retired_deliveries, 1);
        publish(&mut journal, storage(std::slice::from_ref(&old), "parked"));
        journal.prune(|recipient| recipient == &old);
        assert_eq!(journal.len(), 1, "parked retained authority keeps custody");
        assert!(
            journal
                .poll(successor, Channel::Storage, 0, 32, |_| true)
                .unwrap()
                .is_empty()
        );
        journal.retire_page(old.page);
        assert!(journal.is_empty());
        assert!(journal.cursors.is_empty());
    }
    #[test]
    fn diagnostic_ring_reports_its_pruned_history() {
        let mut journal = Journal::default();
        for _ in 0..DIAGNOSTIC_EVENTS + 10 {
            journal.record_diagnostic(ContextEvent::CspReport {
                target_page_id: PageId(1),
                endpoint: "https://example.test/report".into(),
                format: crate::csp::CspReportFormat::LegacyReportUri,
                outcome: "delivered".into(),
            });
        }
        assert_eq!(journal.len(), DIAGNOSTIC_EVENTS);
        assert_eq!(journal.evidence.diagnostics_pruned, 10);
        assert!(journal.evidence.diagnostic_bytes_pruned > 0);
        assert_eq!(journal[0].event_id, 11);
    }
    #[test]
    fn oversize_event_is_rejected_without_an_author_prefix() {
        let mut journal = Journal::default();
        let a = recipient(1, 1);
        let value = "\0".repeat(MAX_EVENT_BYTES / 6 + 1);
        assert!(
            journal
                .prepare(vec![
                    storage(std::slice::from_ref(&a), "prefix"),
                    storage(&[a], &value)
                ])
                .is_err()
        );
        assert!(journal.is_empty());
        assert_eq!(journal.next, 1);
    }
}
