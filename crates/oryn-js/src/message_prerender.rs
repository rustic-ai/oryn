//! Nonactive root authority. A reservation does not replace the visible root.
use super::*;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrerenderToken([u8; 32]);
impl std::fmt::Debug for PrerenderToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrerenderToken([redacted])")
    }
}
pub(super) struct PrerenderDocument {
    pub(super) page: u64,
    initiator: (u64, u64),
    pub(super) lifetime: (u64, u64),
    ready: bool,
    loaded: bool,
    frames: BTreeMap<String, WindowRegistration>,
    queues: BTreeMap<WindowTarget, VecDeque<QueuedWindow>>,
    bytes: BTreeMap<WindowTarget, usize>,
}

impl MessageHub {
    pub(super) fn registered_frame_count(&self) -> usize {
        self.frames
            .len()
            .saturating_add(
                self.parked
                    .values()
                    .map(|document| document.frames.len())
                    .sum::<usize>(),
            )
            .saturating_add(
                self.prerenders
                    .values()
                    .map(|document| document.frames.len())
                    .sum::<usize>(),
            )
    }
    /// Only the parent loader may issue a scope after policy admission. The
    /// native caller owns the matching resource/policy record independently.
    pub fn reserve_prerender_document(
        &mut self,
        page: u64,
        initiator: (u64, u64),
        lifetime: (u64, u64),
    ) -> Result<PrerenderToken, String> {
        if self.roots.get(&page) != Some(&initiator) {
            return Err("prerender initiator is not current".into());
        }
        if lifetime <= initiator
            || self
                .prerender_issued
                .get(&page)
                .is_some_and(|issued| lifetime <= *issued)
            || self.prerenders.len() >= 16
            || self
                .prerenders
                .values()
                .any(|record| record.page == page && record.lifetime == lifetime)
            || self
                .parked
                .values()
                .any(|record| record.page == page && record.lifetime == lifetime)
        {
            return Err("invalid or exhausted prerender authority".into());
        }
        let token = mint_document_capability(
            &mut self.prerender_token_key,
            &mut self.prerender_token_sequence,
            page,
        )?;
        if self.prerenders.contains_key(&token) {
            return Err("prerender token collision".into());
        }
        self.prerender_issued.insert(page, lifetime);
        self.prerenders.insert(
            token,
            PrerenderDocument {
                page,
                initiator,
                lifetime,
                ready: false,
                loaded: false,
                frames: BTreeMap::new(),
                queues: BTreeMap::new(),
                bytes: BTreeMap::new(),
            },
        );
        Ok(PrerenderToken(token))
    }
    pub fn prerender_document_authority(
        &self,
        page: u64,
        token: &PrerenderToken,
        lifetime: (u64, u64),
    ) -> Result<(), String> {
        let record = self
            .prerenders
            .get(&token.0)
            .ok_or("prerender scope retired")?;
        if record.page != page
            || record.lifetime != lifetime
            || self.roots.get(&page) != Some(&record.initiator)
        {
            return Err("prerender source authority changed".into());
        }
        Ok(())
    }
    /// Execute a document-scoped message operation against a private root's
    /// queues and origins under the caller's exclusive hub lock. No callbacks
    /// or external transport occur while the projection is installed.
    pub fn handle_prerender(
        &mut self,
        page: u64,
        token: &PrerenderToken,
        lifetime: (u64, u64),
        request: MessageRequest,
        live: &BTreeMap<u64, String>,
        keys: &BTreeMap<u64, String>,
        origin: &str,
        origin_key: &str,
        partition: String,
    ) -> Result<MessageResponse, String> {
        self.prerender_document_authority(page, token, lifetime)?;
        let MessageRequest::InDocument {
            target,
            document_generation,
            stream_epoch,
            ..
        } = &request
        else {
            return Err("prerender requires exact document message scope".into());
        };
        self.prerender_target_authority(
            page,
            token,
            lifetime,
            target,
            *document_generation,
            *stream_epoch,
        )?;
        self.with_prerender_projection(
            page, token, lifetime, request, live, keys, origin, origin_key, partition,
        )
    }
    /// Native parent caller only; the creator is validated against this exact
    /// private tree before the child can enter its registration table.
    pub fn register_prerender_frame(
        &mut self,
        page: u64,
        token: &PrerenderToken,
        lifetime: (u64, u64),
        parent: &WindowTarget,
        parent_generation: u64,
        parent_epoch: u64,
        frame: String,
        generation: u64,
        epoch: u64,
        frame_origin: String,
        inherit_origin: Option<WindowTarget>,
        origin: &str,
        origin_key: &str,
        partition: String,
    ) -> Result<(), String> {
        self.prerender_target_authority(
            page,
            token,
            lifetime,
            parent,
            parent_generation,
            parent_epoch,
        )?;
        if inherit_origin
            .as_ref()
            .is_some_and(|inherited| inherited != parent)
        {
            return Err("prerender frame origin inheritance must name its actual parent".into());
        }
        let existing = self
            .prerenders
            .get(&token.0)
            .is_some_and(|candidate| candidate.frames.contains_key(&frame));
        let registered = self.registered_frame_count();
        if !existing && registered >= MAX_PORTS {
            return Err("frame registration capacity".into());
        }
        let request = MessageRequest::RegisterWindow {
            frame,
            document_generation: generation,
            stream_epoch: epoch,
            origin: frame_origin,
            inherit_origin,
        };
        let result = self.with_prerender_projection(
            page,
            token,
            lifetime,
            request,
            &BTreeMap::from([(page, origin.into())]),
            &BTreeMap::from([(page, origin_key.into())]),
            origin,
            origin_key,
            partition,
        )?;
        if !matches!(result, MessageResponse::Done) {
            return Err("prerender frame registration response mismatch".into());
        }
        Ok(())
    }
    pub fn prerender_scope_for_document(
        &self,
        page: u64,
        target: &WindowTarget,
        generation: u64,
        epoch: u64,
    ) -> Option<(PrerenderToken, (u64, u64))> {
        self.prerenders.iter().find_map(|(token, candidate)| {
            if candidate.page != page {
                return None;
            }
            let matches = match target {
                WindowTarget::Root => candidate.lifetime == (generation, epoch),
                WindowTarget::Frame(frame) => candidate.frames.get(frame).is_some_and(|entry| {
                    (entry.document_generation, entry.stream_epoch) == (generation, epoch)
                }),
            };
            matches.then_some((PrerenderToken(*token), candidate.lifetime))
        })
    }
    pub fn prerender_target_authority(
        &self,
        page: u64,
        token: &PrerenderToken,
        lifetime: (u64, u64),
        target: &WindowTarget,
        generation: u64,
        epoch: u64,
    ) -> Result<Option<(String, String)>, String> {
        self.prerender_document_authority(page, token, lifetime)?;
        match target {
            WindowTarget::Root if lifetime == (generation, epoch) => Ok(None),
            WindowTarget::Frame(frame) => self
                .prerenders
                .get(&token.0)
                .and_then(|candidate| candidate.frames.get(frame))
                .filter(|entry| {
                    (entry.document_generation, entry.stream_epoch) == (generation, epoch)
                })
                .map(|entry| Some((entry.origin.clone(), entry.origin_key.clone())))
                .ok_or_else(|| "prerender child lifetime mismatch".into()),
            _ => Err("prerender root lifetime mismatch".into()),
        }
    }
    /// Native disposal is valid even when normal creator admission has been
    /// revoked. It removes only the captured child lifetime, never a successor.
    pub fn retire_prerender_frame(
        &mut self,
        page: u64,
        frame: &str,
        generation: u64,
        epoch: u64,
    ) -> bool {
        let target = WindowTarget::Frame(frame.into());
        let Some((token, _)) = self.prerender_scope_for_document(page, &target, generation, epoch)
        else {
            return false;
        };
        let candidate = self.prerenders.get_mut(&token.0).unwrap();
        candidate.frames.remove(frame);
        candidate.bytes.remove(&target);
        let queue = candidate.queues.remove(&target);
        self.retire_broadcast_document(page, target, generation, epoch);
        if let Some(queue) = queue {
            for item in queue {
                self.discard(item.message.payload);
            }
        }
        true
    }
    pub fn commit_prerender_frame_origin(
        &mut self,
        page: u64,
        token: &PrerenderToken,
        lifetime: (u64, u64),
        frame: &str,
        generation: u64,
        epoch: u64,
        origin: (String, String),
    ) -> Result<(), String> {
        self.prerender_target_authority(
            page,
            token,
            lifetime,
            &WindowTarget::Frame(frame.into()),
            generation,
            epoch,
        )?;
        let entry = self
            .prerenders
            .get_mut(&token.0)
            .unwrap()
            .frames
            .get_mut(frame)
            .unwrap();
        entry.origin = origin.0;
        entry.origin_key = origin.1;
        Ok(())
    }
    fn with_prerender_projection(
        &mut self,
        page: u64,
        token: &PrerenderToken,
        lifetime: (u64, u64),
        request: MessageRequest,
        live: &BTreeMap<u64, String>,
        keys: &BTreeMap<u64, String>,
        origin: &str,
        origin_key: &str,
        partition: String,
    ) -> Result<MessageResponse, String> {
        self.prerender_document_authority(page, token, lifetime)?;
        let old_root = self
            .roots
            .insert(page, lifetime)
            .expect("validated prerender initiator");
        let mut active_frames = BTreeMap::new();
        let mut active_queues = BTreeMap::new();
        let mut active_bytes = BTreeMap::new();
        for key in self
            .frames
            .keys()
            .filter(|(owner, _)| *owner == page)
            .cloned()
            .collect::<Vec<_>>()
        {
            active_frames.insert(key.1.clone(), self.frames.remove(&key).unwrap());
        }
        for key in self
            .windows
            .keys()
            .filter(|(owner, _)| *owner == page)
            .cloned()
            .collect::<Vec<_>>()
        {
            active_queues.insert(key.1.clone(), self.windows.remove(&key).unwrap());
        }
        for key in self
            .window_bytes
            .keys()
            .filter(|(owner, _)| *owner == page)
            .cloned()
            .collect::<Vec<_>>()
        {
            active_bytes.insert(key.1.clone(), self.window_bytes.remove(&key).unwrap());
        }
        let candidate = self.prerenders.get_mut(&token.0).unwrap();
        for (frame, value) in std::mem::take(&mut candidate.frames) {
            self.frames.insert((page, frame), value);
        }
        for (target, value) in std::mem::take(&mut candidate.queues) {
            self.windows.insert((page, target), value);
        }
        for (target, value) in std::mem::take(&mut candidate.bytes) {
            self.window_bytes.insert((page, target), value);
        }
        let old_partition = self.partitions.insert(Principal::Window(page), partition);
        let mut live = live.clone();
        live.insert(page, origin.into());
        let mut keys = keys.clone();
        keys.insert(page, origin_key.into());
        let result = self.handle_with_origin_keys(page, request, &live, &keys);
        let candidate = self.prerenders.get_mut(&token.0).unwrap();
        for key in self
            .frames
            .keys()
            .filter(|(owner, _)| *owner == page)
            .cloned()
            .collect::<Vec<_>>()
        {
            candidate
                .frames
                .insert(key.1.clone(), self.frames.remove(&key).unwrap());
        }
        for key in self
            .windows
            .keys()
            .filter(|(owner, _)| *owner == page)
            .cloned()
            .collect::<Vec<_>>()
        {
            candidate
                .queues
                .insert(key.1.clone(), self.windows.remove(&key).unwrap());
        }
        for key in self
            .window_bytes
            .keys()
            .filter(|(owner, _)| *owner == page)
            .cloned()
            .collect::<Vec<_>>()
        {
            candidate
                .bytes
                .insert(key.1.clone(), self.window_bytes.remove(&key).unwrap());
        }
        for (frame, value) in active_frames {
            self.frames.insert((page, frame), value);
        }
        for (target, value) in active_queues {
            self.windows.insert((page, target), value);
        }
        for (target, value) in active_bytes {
            self.window_bytes.insert((page, target), value);
        }
        self.roots.insert(page, old_root);
        match old_partition {
            Some(value) => {
                self.partitions.insert(Principal::Window(page), value);
            }
            None => {
                self.partitions.remove(&Principal::Window(page));
            }
        }
        result
    }

    /// Native Document creation, distinct from load completion. The original
    /// response/parser owner can move into the visible traversable while busy.
    pub fn mark_prerender_ready(
        &mut self,
        page: u64,
        token: &PrerenderToken,
        lifetime: (u64, u64),
    ) -> Result<(), String> {
        self.prerender_document_authority(page, token, lifetime)?;
        self.prerenders.get_mut(&token.0).unwrap().ready = true;
        Ok(())
    }
    pub fn mark_prerender_loaded(
        &mut self,
        page: u64,
        token: &PrerenderToken,
        lifetime: (u64, u64),
    ) -> Result<(), String> {
        self.prerender_document_authority(page, token, lifetime)?;
        let record = self.prerenders.get_mut(&token.0).unwrap();
        record.ready = true;
        record.loaded = true;
        Ok(())
    }
    /// Incumbent callbacks/freeze and destination policy validation precede
    /// this atomic hub operation. No new Document or resource owner is made.
    pub fn promote_prerender_document(
        &mut self,
        page: u64,
        token: &PrerenderToken,
        lifetime: (u64, u64),
    ) -> Result<ParkedDocumentToken, String> {
        self.prerender_document_authority(page, token, lifetime)?;
        let record = &self.prerenders[&token.0];
        if !record.ready {
            return Err("prerender Document has not been created".into());
        }
        if self.frames.len().saturating_add(record.frames.len()) > MAX_PORTS {
            return Err("prerender frame registration capacity".into());
        }
        let initiator = record.initiator;
        // All fallible work precedes consuming the candidate. Parking owns
        // its original queues/frames, and its token supports rollback.
        let incumbent = self.park_document_authority(page, initiator.0, initiator.1)?;
        self.promote_prerender_after_departure(page, token, lifetime, &incumbent)?;
        Ok(incumbent)
    }
    /// Departure already owns pagehide/pageswap and the incumbent cache token.
    /// Validate that exact parked source, without replaying departure or taking
    /// another token. Every rejection leaves both native owners available for
    /// the caller's rollback; a successfully installed newer root wins.
    pub fn promote_prerender_after_departure(
        &mut self,
        page: u64,
        token: &PrerenderToken,
        lifetime: (u64, u64),
        incumbent: &ParkedDocumentToken,
    ) -> Result<(), String> {
        let candidate = self
            .prerenders
            .get(&token.0)
            .ok_or("prerender scope retired")?;
        let source = self
            .parked
            .get(&incumbent.0)
            .ok_or("prerender departure owner retired")?;
        if candidate.page != page
            || candidate.lifetime != lifetime
            || !candidate.ready
            || source.page != page
            || source.frame.is_some()
            || source.lifetime != candidate.initiator
        {
            return Err("prerender departure authority mismatch".into());
        }
        if self.roots.contains_key(&page)
            || self.frames.keys().any(|(owner, _)| *owner == page)
            || self
                .windows
                .iter()
                .any(|((owner, _), queue)| *owner == page && !queue.is_empty())
        {
            return Err("prerender destination already replaced".into());
        }
        if self.frames.len().saturating_add(candidate.frames.len()) > MAX_PORTS {
            return Err("prerender frame registration capacity".into());
        }
        let record = self.prerenders.remove(&token.0).unwrap();
        self.roots.insert(page, record.lifetime);
        for (frame, registration) in record.frames {
            self.frames.insert((page, frame), registration);
        }
        for (target, queue) in record.queues {
            self.windows.insert((page, target), queue);
        }
        for (target, bytes) in record.bytes {
            self.window_bytes.insert((page, target), bytes);
        }
        Ok(())
    }
    pub fn cancel_prerender_document(
        &mut self,
        page: u64,
        token: &PrerenderToken,
    ) -> Result<(), String> {
        let Some(record) = self.prerenders.get(&token.0) else {
            // Disposal acknowledgement only: never proves promotion occurred
            // or grants authority to a document that has since replaced it.
            return if self.prerender_token_consumed(page, token) {
                Ok(())
            } else {
                Err("unknown or foreign prerender capability".into())
            };
        };
        if record.page != page {
            return Err("foreign prerender capability".into());
        }
        let record = self.prerenders.remove(&token.0).unwrap();
        self.retire_broadcast_document(
            page,
            WindowTarget::Root,
            record.lifetime.0,
            record.lifetime.1,
        );
        for (frame, registration) in record.frames {
            self.retire_broadcast_document(
                page,
                WindowTarget::Frame(frame),
                registration.document_generation,
                registration.stream_epoch,
            );
        }
        for queue in record.queues.into_values() {
            for item in queue {
                self.discard(item.message.payload);
            }
        }
        Ok(())
    }
    pub fn prerender_token_consumed(&self, page: u64, token: &PrerenderToken) -> bool {
        authentic_document_capability(
            self.prerender_token_key.as_ref(),
            self.prerender_token_sequence,
            page,
            &token.0,
        ) && !self.prerenders.contains_key(&token.0)
    }
    pub(super) fn cancel_page_prerenders(&mut self, page: u64) {
        self.prerender_issued.remove(&page);
        let tokens = self
            .prerenders
            .iter()
            .filter(|(_, record)| record.page == page)
            .map(|(token, _)| PrerenderToken(*token))
            .collect::<Vec<_>>();
        for token in tokens {
            let _ = self.cancel_prerender_document(page, &token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_child_registration_origin_and_retirement_never_touch_visible_frames() {
        let mut hub = MessageHub::default();
        hub.roots.insert(1, (1, 0));
        let token = hub.reserve_prerender_document(1, (1, 0), (2, 1)).unwrap();
        let frame = "a".repeat(64);
        hub.register_prerender_frame(
            1,
            &token,
            (2, 1),
            &WindowTarget::Root,
            2,
            1,
            frame.clone(),
            1,
            0,
            "https://private.test".into(),
            Some(WindowTarget::Root),
            "https://private.test",
            "private-origin",
            "partition".into(),
        )
        .unwrap();
        assert!(hub.frame_document_authority(1, &frame, 1, 0).is_err());
        assert_eq!(
            hub.prerender_target_authority(
                1,
                &token,
                (2, 1),
                &WindowTarget::Frame(frame.clone()),
                1,
                0
            )
            .unwrap(),
            Some(("https://private.test".into(), "private-origin".into()))
        );
        assert!(
            hub.register_prerender_frame(
                1,
                &token,
                (2, 1),
                &WindowTarget::Root,
                1,
                0,
                "b".repeat(64),
                1,
                0,
                "https://private.test".into(),
                None,
                "https://private.test",
                "private-origin",
                "partition".into()
            )
            .is_err()
        );
        assert!(!hub.retire_prerender_frame(1, &frame, 0, 0));
        assert!(hub.retire_prerender_frame(1, &frame, 1, 0));
        assert!(!hub.retire_prerender_frame(1, &frame, 1, 0));
        assert_eq!(hub.root_document_identity(1), Some((1, 0)));
    }
    #[test]
    fn private_self_messages_preserve_visible_queues_and_restore_projection_after_error() {
        let mut hub = MessageHub::default();
        hub.roots.insert(1, (1, 0));
        let token = hub.reserve_prerender_document(1, (1, 0), (2, 1)).unwrap();
        let live = BTreeMap::from([(1, "https://source.test".into())]);
        let keys = BTreeMap::from([(1, "source-origin".into())]);
        let payload = || WireValue {
            version: 1,
            bytes: vec![7],
            hosts: vec![],
            buffers: vec![],
            ports: vec![],
            shared: vec![],
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: vec![],
        };
        let scoped = |request| MessageRequest::InDocument {
            target: WindowTarget::Root,
            document_generation: 2,
            stream_epoch: 1,
            request: Box::new(request),
        };
        let post = || MessageRequest::PostWindow {
            target_page: 1,
            target: WindowTarget::Root,
            source: WindowTarget::Root,
            target_origin: TargetOrigin::Any,
            payload: payload(),
        };
        hub.handle_with_origin_keys(1, post(), &live, &keys)
            .unwrap();
        hub.handle_prerender(
            1,
            &token,
            (2, 1),
            scoped(post()),
            &live,
            &keys,
            "null",
            "private-opaque",
            "private-partition".into(),
        )
        .unwrap();
        assert_eq!(hub.root_document_identity(1), Some((1, 0)));
        let MessageResponse::WindowMessages(visible) = hub
            .handle_with_origin_keys(
                1,
                MessageRequest::PollWindow {
                    target: WindowTarget::Root,
                    max: 8,
                },
                &live,
                &keys,
            )
            .unwrap()
        else {
            panic!("visible queue")
        };
        assert_eq!(visible.len(), 1);
        let result = hub.handle_prerender(
            1,
            &token,
            (2, 1),
            scoped(MessageRequest::RegisterRootDocument {
                document_generation: 99,
                stream_epoch: 0,
            }),
            &live,
            &keys,
            "null",
            "private-opaque",
            "private-partition".into(),
        );
        assert!(result.is_err());
        assert_eq!(hub.root_document_identity(1), Some((1, 0)));
        let MessageResponse::WindowMessages(private) = hub
            .handle_prerender(
                1,
                &token,
                (2, 1),
                scoped(MessageRequest::PollWindow {
                    target: WindowTarget::Root,
                    max: 8,
                }),
                &live,
                &keys,
                "null",
                "private-opaque",
                "private-partition".into(),
            )
            .unwrap()
        else {
            panic!("private queue")
        };
        assert_eq!(private.len(), 1);
        assert_eq!(private[0].source_origin, "null");
        assert_eq!(hub.root_document_identity(1), Some((1, 0)));
    }
    #[test]
    fn visible_registration_accounts_for_owned_private_frame_capacity() {
        let mut hub = MessageHub::default();
        hub.roots.insert(1, (1, 0));
        let token = hub.reserve_prerender_document(1, (1, 0), (2, 1)).unwrap();
        let private = hub.prerenders.get_mut(&token.0).unwrap();
        for index in 0..MAX_PORTS {
            private.frames.insert(
                format!("{index:064x}"),
                WindowRegistration {
                    document_generation: 2,
                    stream_epoch: 1,
                    origin: "https://example.test".into(),
                    origin_key: "https://example.test".into(),
                },
            );
        }
        assert_eq!(hub.registered_frame_count(), MAX_PORTS);
        let live = BTreeMap::from([(1, "https://example.test".into())]);
        let result = hub.handle_with_origin_keys(
            1,
            MessageRequest::RegisterWindow {
                frame: "f".repeat(64),
                document_generation: 1,
                stream_epoch: 0,
                origin: "https://example.test".into(),
                inherit_origin: None,
            },
            &live,
            &live,
        );
        assert!(result.is_err());
        assert!(hub.frames.is_empty());
        hub.prerenders.get_mut(&token.0).unwrap().frames.pop_first();
        assert!(
            hub.handle_with_origin_keys(
                1,
                MessageRequest::RegisterWindow {
                    frame: "f".repeat(64),
                    document_generation: 1,
                    stream_epoch: 0,
                    origin: "https://example.test".into(),
                    inherit_origin: None
                },
                &live,
                &live
            )
            .is_ok()
        );
        assert_eq!(hub.registered_frame_count(), MAX_PORTS);
    }
    #[test]
    fn created_document_can_promote_before_load_and_foreign_ready_cannot_admit() {
        let mut hub = MessageHub::default();
        hub.roots.insert(1, (1, 0));
        let token = hub.reserve_prerender_document(1, (1, 0), (2, 1)).unwrap();
        assert!(hub.mark_prerender_ready(2, &token, (2, 1)).is_err());
        assert!(hub.mark_prerender_ready(1, &token, (2, 2)).is_err());
        hub.mark_prerender_ready(1, &token, (2, 1)).unwrap();
        assert!(!hub.prerenders[&token.0].loaded);
        hub.promote_prerender_document(1, &token, (2, 1)).unwrap();
        assert_eq!(hub.roots.get(&1), Some(&(2, 1)));
    }
    #[test]
    fn nonactive_root_promotion_preserves_incumbent_until_exact_loaded_admission() {
        let mut hub = MessageHub::default();
        hub.roots.insert(1, (1, 0));
        let token = hub.reserve_prerender_document(1, (1, 0), (2, 0)).unwrap();
        assert_eq!(hub.root_document_identity(1), Some((1, 0)));
        assert!(hub.promote_prerender_document(1, &token, (2, 0)).is_err());
        assert!(hub.mark_prerender_loaded(2, &token, (2, 0)).is_err());
        hub.mark_prerender_loaded(1, &token, (2, 0)).unwrap();
        let old = hub.promote_prerender_document(1, &token, (2, 0)).unwrap();
        assert_eq!(hub.root_document_identity(1), Some((2, 0)));
        assert!(hub.prerender_document_authority(1, &token, (2, 0)).is_err());
        let new = hub.park_document_authority(1, 2, 0).unwrap();
        hub.restore_document_authority(1, &old).unwrap();
        hub.discard_parked_document(1, &new).unwrap();
        assert_eq!(hub.root_document_identity(1), Some((1, 0)));
    }
    #[test]
    fn prepared_departure_promotes_once_and_preserves_source_rollback_token() {
        let mut hub = MessageHub::default();
        hub.roots.insert(1, (1, 0));
        let candidate = hub.reserve_prerender_document(1, (1, 0), (2, 1)).unwrap();
        hub.mark_prerender_loaded(1, &candidate, (2, 1)).unwrap();
        let departure = hub.park_document_authority(1, 1, 0).unwrap();
        assert!(
            hub.promote_prerender_after_departure(2, &candidate, (2, 1), &departure)
                .is_err()
        );
        assert!(
            hub.promote_prerender_after_departure(1, &candidate, (2, 2), &departure)
                .is_err()
        );
        assert!(hub.prerenders.contains_key(&candidate.0));
        assert!(hub.parked.contains_key(&departure.0));
        hub.promote_prerender_after_departure(1, &candidate, (2, 1), &departure)
            .unwrap();
        assert_eq!(hub.root_document_identity(1), Some((2, 1)));
        assert!(
            hub.promote_prerender_after_departure(1, &candidate, (2, 1), &departure)
                .is_err()
        );
        let activated = hub.park_document_authority(1, 2, 1).unwrap();
        hub.restore_document_authority(1, &departure).unwrap();
        hub.discard_parked_document(1, &activated).unwrap();
        assert_eq!(hub.root_document_identity(1), Some((1, 0)));
    }
    #[test]
    fn newer_root_prevents_prerender_promotion_after_departure() {
        let mut hub = MessageHub::default();
        hub.roots.insert(1, (1, 0));
        let candidate = hub.reserve_prerender_document(1, (1, 0), (2, 1)).unwrap();
        hub.mark_prerender_loaded(1, &candidate, (2, 1)).unwrap();
        let departure = hub.park_document_authority(1, 1, 0).unwrap();
        hub.roots.insert(1, (3, 2));
        assert!(
            hub.promote_prerender_after_departure(1, &candidate, (2, 1), &departure)
                .is_err()
        );
        assert_eq!(hub.root_document_identity(1), Some((3, 2)));
        assert!(hub.prerenders.contains_key(&candidate.0));
        hub.cancel_prerender_document(1, &candidate).unwrap();
        assert_eq!(hub.root_document_identity(1), Some((3, 2)));
    }
    #[test]
    fn consumed_prerender_disposal_is_authenticated_bounded_and_not_activation() {
        let mut hub = MessageHub::default();
        hub.roots.insert(1, (1, 0));
        let token = hub.reserve_prerender_document(1, (1, 0), (2, 1)).unwrap();
        assert!(!hub.prerender_token_consumed(1, &token));
        hub.cancel_prerender_document(1, &token).unwrap();
        for _ in 0..3 {
            hub.cancel_prerender_document(1, &token).unwrap();
        }
        assert_eq!(hub.root_document_identity(1), Some((1, 0)));
        assert!(hub.mark_prerender_loaded(1, &token, (2, 1)).is_err());
        assert!(hub.cancel_prerender_document(2, &token).is_err());
        let mut forged = token.clone();
        forged.0[20] ^= 1;
        assert!(hub.cancel_prerender_document(1, &forged).is_err());
        let parked = hub.park_document_authority(1, 1, 0).unwrap();
        assert!(
            hub.cancel_prerender_document(1, &PrerenderToken(parked.0))
                .is_err()
        );
        assert!(!hub.cached_token_expired(1, &ParkedDocumentToken(token.0)));
        assert!(hub.prerenders.is_empty());
        hub.restore_document_authority(1, &parked).unwrap();
        hub.prerender_token_sequence = u64::MAX;
        assert!(hub.reserve_prerender_document(1, (1, 0), (3, 2)).is_err());
    }
    #[test]
    fn superseded_initiator_cannot_promote_and_retirement_removes_candidate() {
        let mut hub = MessageHub::default();
        hub.roots.insert(1, (1, 0));
        let token = hub.reserve_prerender_document(1, (1, 0), (2, 0)).unwrap();
        hub.roots.insert(1, (3, 0));
        assert!(hub.mark_prerender_loaded(1, &token, (2, 0)).is_err());
        assert!(hub.promote_prerender_document(1, &token, (2, 0)).is_err());
        hub.retire_page(1);
        assert!(hub.prerenders.is_empty());
    }
}
