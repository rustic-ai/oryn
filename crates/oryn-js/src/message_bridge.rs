//! Parent-only transfer of entanglement between two independently authorized
//! context hubs. No Window or Agent principal is invented for bridge endpoints.
use super::*;

pub struct CrossHubPortBridge {
    left: PortToken,
    right: PortToken,
}
impl std::fmt::Debug for CrossHubPortBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CrossHubPortBridge([native capabilities])")
    }
}

fn collect(
    source: &MessageHub,
    token: &PortToken,
    owner: Owner,
    ports: &mut BTreeMap<u64, (PortToken, Owner)>,
) -> Result<(), String> {
    let mut pending = vec![(token.clone(), owner)];
    while let Some((token, owner)) = pending.pop() {
        if ports.len() >= MAX_PORTS {
            return Err("cross-hub transfer graph capacity".into());
        }
        if ports.contains_key(&token.id) {
            return Err("cross-hub transfer graph repeats endpoint ownership".into());
        }
        let endpoint = source
            .ports
            .get(&token.id)
            .ok_or("cross-hub transferred port missing")?;
        if endpoint.closed || endpoint.secret != token.secret || endpoint.owner != owner {
            return Err("cross-hub transferred port is foreign or stale".into());
        }
        ports.insert(token.id, (token, owner));
        for queued in &endpoint.queue {
            queued.validate()?;
            if !queued.shared.is_empty() {
                return Err("DataCloneError: queued shared backing cannot cross native agent-cluster registries".into());
            }
            for child in &queued.ports {
                pending.push((child.clone(), Owner::Queued));
            }
        }
    }
    Ok(())
}

/// Caller holds the two native context locks in a consistent order and has
/// validated both live principals and their shared storage authority.
pub fn route_cross_hub_value(
    source: &mut MessageHub,
    target: &mut MessageHub,
    sender: Principal,
    recipient: Principal,
    value: &mut WireValue,
) -> Result<Vec<CrossHubPortBridge>, String> {
    migrate(
        source,
        target,
        Owner::Transit(sender),
        Owner::Bound(recipient),
        value,
        0,
    )
}

fn migrate(
    source: &mut MessageHub,
    target: &mut MessageHub,
    owner: Owner,
    recipient: Owner,
    value: &mut WireValue,
    additional_bytes: usize,
) -> Result<Vec<CrossHubPortBridge>, String> {
    value.validate()?;
    if !value.shared.is_empty() {
        return Err(
            "DataCloneError: shared backing cannot cross native agent-cluster registries".into(),
        );
    }
    let mut graph = BTreeMap::new();
    for token in &value.ports {
        collect(source, token, owner, &mut graph)?;
    }
    let required = graph
        .len()
        .checked_mul(2)
        .ok_or("cross-hub port capacity overflow")?;
    if required > MAX_PORTS.saturating_sub(target.ports.len()) {
        return Err("destination MessagePort capacity".into());
    }
    let queued_bytes = graph.keys().try_fold(0usize, |total, id| {
        total
            .checked_add(source.ports[id].bytes)
            .ok_or("cross-hub queue size overflow")
    })?;
    target.queue_budget(
        queued_bytes
            .checked_add(additional_bytes)
            .ok_or("cross-hub queue size overflow")?,
    )?;
    let end = target
        .next
        .checked_add(u64::try_from(required).map_err(|_| "cross-hub identity capacity")?)
        .ok_or("MessagePort identity exhausted")?;
    // Entropy and capacity are fully reserved before changing either hub.
    let mut mappings = BTreeMap::new();
    let mut next = target.next;
    for (id, (_, original_owner)) in &graph {
        next += 1;
        let actual = PortToken {
            id: next,
            secret: secret()?,
        };
        next += 1;
        let proxy = PortToken {
            id: next,
            secret: secret()?,
        };
        let left = PortToken {
            id: *id,
            secret: secret()?,
        };
        let destination_owner = if *original_owner == owner {
            recipient
        } else {
            Owner::Queued
        };
        mappings.insert(*id, (actual, proxy, left, destination_owner));
    }
    let mut bridges = Vec::with_capacity(graph.len());
    for id in graph.keys() {
        let mut old = source
            .ports
            .remove(id)
            .expect("preflight retained transferred endpoint");
        let (actual, proxy, left, destination_owner) = &mappings[id];
        for queued in &mut old.queue {
            for token in &mut queued.ports {
                *token = mappings[&token.id].0.clone();
            }
        }
        let original_peer = old.peer.take();
        let actual_endpoint = Endpoint {
            secret: actual.secret,
            owner: *destination_owner,
            peer: Some(proxy.id),
            closed: false,
            close_pending: old.close_pending,
            queue: old.queue,
            bytes: old.bytes,
        };
        source.ports.insert(
            *id,
            Endpoint {
                secret: left.secret,
                owner: Owner::Bridge,
                peer: original_peer,
                closed: false,
                close_pending: false,
                queue: VecDeque::new(),
                bytes: 0,
            },
        );
        target.ports.insert(actual.id, actual_endpoint);
        target.ports.insert(
            proxy.id,
            Endpoint {
                secret: proxy.secret,
                owner: Owner::Bridge,
                peer: Some(actual.id),
                closed: false,
                close_pending: false,
                queue: VecDeque::new(),
                bytes: 0,
            },
        );
        bridges.push(CrossHubPortBridge {
            left: left.clone(),
            right: proxy.clone(),
        });
    }
    target.next = end;
    for token in &mut value.ports {
        *token = mappings[&token.id].0.clone();
    }
    Ok(bridges)
}

fn bridge_endpoint<'a>(
    hub: &'a MessageHub,
    token: &PortToken,
) -> Result<Option<&'a Endpoint>, String> {
    let Some(endpoint) = hub.ports.get(&token.id) else {
        return Ok(None);
    };
    if endpoint.owner != Owner::Bridge || endpoint.secret != token.secret {
        return Err("native cross-hub bridge capability mismatch".into());
    }
    Ok(Some(endpoint))
}

impl CrossHubPortBridge {
    /// Forward a bounded batch, preserving FIFO and retaining failed admission
    /// at the source for retry. Nested transferred ports acquire their own
    /// entanglement bridges before delivery becomes visible.
    pub fn pump(
        &self,
        left: &mut MessageHub,
        right: &mut MessageHub,
        nested: &mut Vec<CrossHubPortBridge>,
    ) -> Result<(), String> {
        bridge_endpoint(left, &self.left)?;
        bridge_endpoint(right, &self.right)?;
        // Caller retains all newly admitted bridges even if a later message
        // in this batch fails; an error never abandons earlier deliveries.
        let mut first_error = self
            .forward(left, right, &self.left, &self.right, false, nested)
            .err();
        // Backpressure in one direction must not prevent the reverse endpoint
        // from making progress or completing its already queued transfers.
        if let Err(error) = self.forward(right, left, &self.right, &self.left, true, nested) {
            first_error.get_or_insert(error);
        }
        // Drain messages sent before peer closure before retiring the bridge.
        let a = bridge_endpoint(left, &self.left)?;
        let b = bridge_endpoint(right, &self.right)?;
        let closed = a.is_none_or(|endpoint| endpoint.peer.is_none())
            || b.is_none_or(|endpoint| endpoint.peer.is_none());
        let empty = a.is_none_or(|endpoint| endpoint.queue.is_empty())
            && b.is_none_or(|endpoint| endpoint.queue.is_empty());
        if closed && empty {
            if let Err(error) = self.retire(left, right) {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
    fn forward(
        &self,
        source: &mut MessageHub,
        target: &mut MessageHub,
        source_token: &PortToken,
        target_token: &PortToken,
        reverse: bool,
        nested: &mut Vec<CrossHubPortBridge>,
    ) -> Result<(), String> {
        for _ in 0..32 {
            let Some(endpoint) = bridge_endpoint(source, source_token)? else {
                return Ok(());
            };
            let Some(mut payload) = endpoint.queue.front().cloned() else {
                return Ok(());
            };
            let peer = bridge_endpoint(target, target_token)?.and_then(|endpoint| endpoint.peer);
            let Some(peer) = peer else {
                let payload = source
                    .ports
                    .get_mut(&source_token.id)
                    .unwrap()
                    .queue
                    .pop_front()
                    .unwrap();
                source.ports.get_mut(&source_token.id).unwrap().bytes -= payload.retained_bytes();
                source.discard(payload);
                continue;
            };
            let bytes = payload.retained_bytes();
            target.queue_budget(bytes)?;
            let destination = target
                .ports
                .get(&peer)
                .ok_or("cross-hub destination peer missing")?;
            if destination.queue.len() >= MAX_MESSAGES
                || bytes > MAX_BYTES.saturating_sub(destination.bytes)
            {
                return Err("cross-hub destination queue capacity".into());
            }
            let added = migrate(
                source,
                target,
                Owner::Queued,
                Owner::Queued,
                &mut payload,
                bytes,
            )?;
            // Both capabilities remain locked and validated throughout commit.
            let old = source
                .ports
                .get_mut(&source_token.id)
                .unwrap()
                .queue
                .pop_front()
                .unwrap();
            source.ports.get_mut(&source_token.id).unwrap().bytes -= old.retained_bytes();
            let destination = target.ports.get_mut(&peer).unwrap();
            destination.bytes += bytes;
            destination.queue.push_back(payload);
            nested.extend(added.into_iter().map(|bridge| {
                if reverse {
                    CrossHubPortBridge {
                        left: bridge.right,
                        right: bridge.left,
                    }
                } else {
                    bridge
                }
            }));
        }
        Ok(())
    }
    pub fn retired(&self, left: &MessageHub, right: &MessageHub) -> Result<bool, String> {
        Ok(bridge_endpoint(left, &self.left)?.is_none()
            && bridge_endpoint(right, &self.right)?.is_none())
    }
    pub fn retire_left(&self, hub: &mut MessageHub) -> Result<(), String> {
        bridge_endpoint(hub, &self.left)?;
        hub.close_id(self.left.id);
        Ok(())
    }
    pub fn retire_right(&self, hub: &mut MessageHub) -> Result<(), String> {
        bridge_endpoint(hub, &self.right)?;
        hub.close_id(self.right.id);
        Ok(())
    }
    pub fn retire(&self, left: &mut MessageHub, right: &mut MessageHub) -> Result<(), String> {
        let mut first_error = None;
        match bridge_endpoint(left, &self.left) {
            Ok(_) => left.close_id(self.left.id),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
        match bridge_endpoint(right, &self.right) {
            Ok(_) => right.close_id(self.right.id),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn value(ports: Vec<PortToken>) -> WireValue {
        WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![7],
            hosts: Vec::new(),
            buffers: Vec::new(),
            ports,
        }
    }
    fn request(
        hub: &mut MessageHub,
        agent: u64,
        request: MessageRequest,
    ) -> Result<MessageResponse, String> {
        hub.handle_agent(
            agent,
            request,
            "https://example.test",
            "oryn:origin:https://example.test",
        )
    }
    fn transfer(hub: &mut MessageHub, agent: u64, port: PortToken) -> PortToken {
        let MessageResponse::Transferred(mut ports) =
            request(hub, agent, MessageRequest::Transfer { ports: vec![port] }).unwrap()
        else {
            panic!("transfer")
        };
        ports.remove(0)
    }
    fn receive(hub: &mut MessageHub, agent: u64, port: PortToken) -> (Vec<WireValue>, bool) {
        let MessageResponse::PortMessages {
            messages,
            peer_closed,
        } = request(hub, agent, MessageRequest::PollPort { port, max: 32 }).unwrap()
        else {
            panic!("receive")
        };
        (messages, peer_closed)
    }
    #[test]
    fn backpressure_does_not_starve_reverse_messages_and_retirement_attempts_both_sides() {
        let mut left = MessageHub::default();
        let mut right = MessageHub::default();
        let [a, b] = left
            .create_entangled(Principal::Agent(1), Principal::Agent(1))
            .unwrap();
        let ticket = transfer(&mut left, 1, b);
        let mut payload = value(vec![ticket]);
        let bridges = route_cross_hub_value(
            &mut left,
            &mut right,
            Principal::Agent(1),
            Principal::Agent(2),
            &mut payload,
        )
        .unwrap();
        let received = payload.ports.remove(0);
        let bridge = &bridges[0];
        // Fill just the receiving queue. The reverse direction remains usable.
        for _ in 0..MAX_MESSAGES {
            let message = value(Vec::new());
            let endpoint = right.ports.get_mut(&received.id).unwrap();
            endpoint.bytes += message.retained_bytes();
            endpoint.queue.push_back(message);
        }
        request(
            &mut left,
            1,
            MessageRequest::PostPort {
                port: a.clone(),
                payload: value(Vec::new()),
            },
        )
        .unwrap();
        request(
            &mut right,
            2,
            MessageRequest::PostPort {
                port: received.clone(),
                payload: value(Vec::new()),
            },
        )
        .unwrap();
        let mut nested = Vec::new();
        assert!(bridge.pump(&mut left, &mut right, &mut nested).is_err());
        assert_eq!(
            receive(&mut left, 1, a.clone()).0.len(),
            1,
            "reverse direction still progresses"
        );
        assert_eq!(
            left.ports[&bridge.left.id].queue.len(),
            1,
            "blocked message remains owned"
        );
        let original = left.ports[&bridge.left.id].secret;
        left.ports.get_mut(&bridge.left.id).unwrap().secret = secret().unwrap();
        assert!(bridge.retire(&mut left, &mut right).is_err());
        assert!(
            !right.ports.contains_key(&bridge.right.id),
            "valid peer is retired despite other-side failure"
        );
        assert!(
            left.ports.contains_key(&bridge.left.id),
            "mismatching capability is never closed"
        );
        left.ports.get_mut(&bridge.left.id).unwrap().secret = original;
        bridge.retire(&mut left, &mut right).unwrap();
        assert!(bridge.retired(&left, &right).unwrap());
    }
    #[test]
    fn cross_hub_entanglement_preserves_nested_transfers_and_close() {
        let mut left = MessageHub::default();
        let mut right = MessageHub::default();
        let [a, b] = left
            .create_entangled(Principal::Agent(1), Principal::Agent(1))
            .unwrap();
        let ticket = transfer(&mut left, 1, b);
        let mut payload = value(vec![ticket.clone()]);
        let bridges = route_cross_hub_value(
            &mut left,
            &mut right,
            Principal::Agent(1),
            Principal::Agent(2),
            &mut payload,
        )
        .unwrap();
        let received = payload.ports.remove(0);
        assert!(
            request(
                &mut left,
                1,
                MessageRequest::PollPort {
                    port: ticket,
                    max: 1
                }
            )
            .is_err()
        );
        request(
            &mut left,
            1,
            MessageRequest::PostPort {
                port: a.clone(),
                payload: value(Vec::new()),
            },
        )
        .unwrap();
        let mut nested = Vec::new();
        bridges[0].pump(&mut left, &mut right, &mut nested).unwrap();
        assert_eq!(receive(&mut right, 2, received.clone()).0.len(), 1);
        let [c, d] = right
            .create_entangled(Principal::Agent(2), Principal::Agent(2))
            .unwrap();
        let ticket = transfer(&mut right, 2, d);
        request(
            &mut right,
            2,
            MessageRequest::PostPort {
                port: received.clone(),
                payload: value(vec![ticket]),
            },
        )
        .unwrap();
        bridges[0].pump(&mut left, &mut right, &mut nested).unwrap();
        assert_eq!(nested.len(), 1);
        let transferred = receive(&mut left, 1, a.clone()).0.remove(0).ports.remove(0);
        request(
            &mut right,
            2,
            MessageRequest::PostPort {
                port: c,
                payload: value(Vec::new()),
            },
        )
        .unwrap();
        let mut deeper = Vec::new();
        nested[0].pump(&mut left, &mut right, &mut deeper).unwrap();
        assert_eq!(receive(&mut left, 1, transferred).0.len(), 1);
        request(&mut left, 1, MessageRequest::ClosePort { port: a }).unwrap();
        bridges[0].pump(&mut left, &mut right, &mut deeper).unwrap();
        assert!(receive(&mut right, 2, received).1);
        assert!(bridges[0].retired(&left, &right).unwrap());
        for bridge in nested {
            bridge.retire(&mut left, &mut right).unwrap();
        }
    }
    #[test]
    fn rejected_cross_hub_admission_preserves_transit_and_rejects_foreign_sender() {
        let mut left = MessageHub::default();
        let mut right = MessageHub::default();
        let [_, b] = left
            .create_entangled(Principal::Agent(1), Principal::Agent(1))
            .unwrap();
        let ticket = transfer(&mut left, 1, b);
        let mut payload = value(vec![ticket.clone()]);
        assert!(
            route_cross_hub_value(
                &mut left,
                &mut right,
                Principal::Agent(99),
                Principal::Agent(2),
                &mut payload
            )
            .is_err()
        );
        for _ in 0..MAX_PORTS / 2 {
            right
                .create_entangled(Principal::Agent(2), Principal::Agent(2))
                .unwrap();
        }
        assert!(
            route_cross_hub_value(
                &mut left,
                &mut right,
                Principal::Agent(1),
                Principal::Agent(2),
                &mut payload
            )
            .is_err()
        );
        assert_eq!(payload.ports[0], ticket);
        assert!(left.endpoint(Principal::Agent(1), &ticket, true).is_ok());
    }
    #[test]
    fn forwarding_failure_retains_previously_admitted_nested_bridges() {
        let mut left = MessageHub::default();
        let mut right = MessageHub::default();
        let [a, b] = left
            .create_entangled(Principal::Agent(1), Principal::Agent(1))
            .unwrap();
        let ticket = transfer(&mut left, 1, b);
        let mut payload = value(vec![ticket]);
        let bridges = route_cross_hub_value(
            &mut left,
            &mut right,
            Principal::Agent(1),
            Principal::Agent(2),
            &mut payload,
        )
        .unwrap();
        let received = payload.ports.remove(0);
        let endpoint = right.ports.get_mut(&received.id).unwrap();
        for _ in 0..MAX_MESSAGES - 1 {
            let payload = value(Vec::new());
            endpoint.bytes += payload.retained_bytes();
            endpoint.queue.push_back(payload);
        }
        let [_, child] = left
            .create_entangled(Principal::Agent(1), Principal::Agent(1))
            .unwrap();
        let child = transfer(&mut left, 1, child);
        request(
            &mut left,
            1,
            MessageRequest::PostPort {
                port: a.clone(),
                payload: value(vec![child]),
            },
        )
        .unwrap();
        request(
            &mut left,
            1,
            MessageRequest::PostPort {
                port: a,
                payload: value(Vec::new()),
            },
        )
        .unwrap();
        let mut admitted = Vec::new();
        assert!(
            bridges[0]
                .pump(&mut left, &mut right, &mut admitted)
                .is_err()
        );
        assert_eq!(
            admitted.len(),
            1,
            "successful earlier transfer must remain owned after later backpressure"
        );
        assert_eq!(
            left.ports[&bridges[0].left.id].queue.len(),
            1,
            "failed delivery remains queued at source"
        );
        for bridge in admitted {
            bridge.retire(&mut left, &mut right).unwrap();
        }
        bridges[0].retire(&mut left, &mut right).unwrap();
    }
}
