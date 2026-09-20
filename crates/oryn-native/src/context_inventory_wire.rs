//! Decode PageId object keys after serde's internally tagged request buffer.
//! Keep the existing JSON object representation and exact numeric identities.
use std::{collections::BTreeMap, fmt, marker::PhantomData};

use oryn_common::v3::PageId;
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, Visitor},
};

pub(super) fn deserialize<'de, D, V>(deserializer: D) -> Result<BTreeMap<PageId, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct Inventory<V>(PhantomData<V>);
    impl<'de, V: Deserialize<'de>> Visitor<'de> for Inventory<V> {
        type Value = BTreeMap<PageId, V>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an inventory keyed by canonical unsigned page identifiers")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
            let mut inventory = BTreeMap::new();
            while let Some(key) = input.next_key::<String>()? {
                let id = key.parse::<u64>().map_err(de::Error::custom)?;
                if key != id.to_string() {
                    return Err(de::Error::custom("noncanonical inventory page identifier"));
                }
                let id = PageId(id);
                if inventory.contains_key(&id) {
                    return Err(de::Error::custom("duplicate inventory page identifier"));
                }
                inventory.insert(id, input.next_value()?);
            }
            Ok(inventory)
        }
    }
    deserializer.deserialize_map(Inventory(PhantomData))
}

#[cfg(test)]
mod tests {
    use super::super::{
        CONTEXT_SERVICE_PROTOCOL_VERSION, ContextServiceRequest, ContextServiceResponse,
        ContextTransport, IpcContextClient, RequestEnvelope, SharedContextClient,
        serve_context_transport,
    };
    use super::*;
    use crate::{RuntimeError, host::webdriver_script::AutomationRealmSnapshot};
    use oryn_js::{message_wire::WindowTarget, realm::RealmId, worker_wire::Creator};
    use std::{
        io::{Read, Write},
        net::Shutdown,
        os::unix::net::UnixStream,
        sync::{Arc, Mutex, mpsc},
        time::Duration,
    };

    type Entries = BTreeMap<PageId, Vec<(String, AutomationRealmSnapshot)>>;
    fn entries() -> Entries {
        BTreeMap::from([
            // The hanging frame fixture publishes this negative root entry.
            (PageId(1), Vec::new()),
            (
                PageId(u64::MAX),
                vec![(
                    "child".into(),
                    AutomationRealmSnapshot {
                        realm: RealmId(7),
                        parent_realm: Some(RealmId(2)),
                        frame: Some(oryn_dom::NodeId {
                            slot: 8,
                            generation: 3,
                        }),
                        frame_generation: Some(13),
                        frame_token: Some("exact-token".into()),
                        document_root: oryn_dom::NodeId {
                            slot: 19,
                            generation: 5,
                        },
                        document_generation: 23,
                        stream_epoch: 29,
                        document_authority_epoch: 31,
                        url: "https://inventory.test/child".into(),
                        live: true,
                    },
                )],
            ),
        ])
    }
    fn requests(inventory: Entries) -> Vec<ContextServiceRequest> {
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 23,
            stream_epoch: 29,
        };
        vec![
            ContextServiceRequest::PublishNativeWindowInventory {
                page_id: PageId(1),
                creator: creator.clone(),
                inventory: inventory.clone(),
            },
            ContextServiceRequest::ReadRelatedWindow {
                source_page_id: PageId(1),
                creator: creator.clone(),
                target_page_id: PageId(2),
                target: WindowTarget::Root,
                expected: None,
                local_inventory: inventory.clone(),
            },
            ContextServiceRequest::OpenPage {
                hyperlink_audit: None,
                local_inventory: inventory,
                link_navigation: false,
                user_initiated: false,
                defer_navigation: false,
                local_related_navigation: false,
                selected_target: None,
                document_source: None,
                opener_page_id: PageId(1),
                noopener: false,
                target_name: "child".into(),
                form_authority: None,
                creator: Some(creator),
                url: Some("https://inventory.test/next".into()),
                post: None,
                referrer_policy: None,
            },
        ]
    }
    #[test]
    fn every_tagged_inventory_request_round_trips_exact_authority_and_negative_entries() {
        for inventory in [BTreeMap::new(), entries()] {
            for request in requests(inventory) {
                let envelope = RequestEnvelope {
                    protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                    route: None,
                    message_id: 17,
                    request,
                };
                let encoded = serde_json::to_vec(&envelope).unwrap();
                let decoded: RequestEnvelope = serde_json::from_slice(&encoded).unwrap();
                assert_eq!(serde_json::to_vec(&decoded).unwrap(), encoded);
            }
        }
        let old_open = r#"{"operation":"open_page","opener_page_id":1,"noopener":false,"target_name":"x","form_authority":null,"url":null}"#;
        let ContextServiceRequest::OpenPage {
            local_inventory,
            local_related_navigation,
            ..
        } = serde_json::from_str(old_open).unwrap()
        else {
            panic!("open page")
        };
        assert!(
            !local_related_navigation,
            "legacy callers retain parent navigation ownership"
        );
        assert!(
            local_inventory.is_empty(),
            "omitted legacy inventory retains its default"
        );
    }
    #[test]
    fn inventory_keys_reject_aliases_overflow_and_duplicates_after_tag_buffering() {
        let encoded =
            serde_json::to_string(&requests(BTreeMap::from([(PageId(1), Vec::new())])).remove(0))
                .unwrap();
        assert!(encoded.contains("\"inventory\":{\"1\":[]}"));
        for invalid in ["-1", "+1", "01", "18446744073709551616", "not-a-page"] {
            let malformed = encoded.replace("\"1\":[]", &format!("{invalid:?}:[]"));
            assert!(
                serde_json::from_str::<ContextServiceRequest>(&malformed).is_err(),
                "{invalid}"
            );
        }
        let duplicate = encoded.replace("\"1\":[]", "\"1\":[],\"1\":[]");
        assert!(serde_json::from_str::<ContextServiceRequest>(&duplicate).is_err());
    }
    struct ShutdownOnDrop(UnixStream);
    impl Drop for ShutdownOnDrop {
        fn drop(&mut self) {
            let _ = self.0.shutdown(Shutdown::Both);
        }
    }
    #[derive(Default)]
    struct Recording(Mutex<Vec<serde_json::Value>>);
    impl ContextTransport for Recording {
        fn request(
            &self,
            request: ContextServiceRequest,
        ) -> Result<ContextServiceResponse, RuntimeError> {
            self.0
                .lock()
                .unwrap()
                .push(serde_json::to_value(request).unwrap());
            Ok(ContextServiceResponse::NativeWindowInventoryPublished)
        }
    }
    #[test]
    fn nonempty_inventories_complete_over_shared_ipc_with_exact_payloads() {
        let (worker, parent) = UnixStream::pair().unwrap();
        let shutdown = ShutdownOnDrop(parent.try_clone().unwrap());
        let service = Arc::new(Recording::default());
        let served = service.clone();
        let server = std::thread::spawn(move || serve_context_transport(parent, served));
        let requests = requests(entries());
        let expected = requests
            .iter()
            .map(|request| serde_json::to_value(request).unwrap())
            .collect::<Vec<_>>();
        let (finished, result) = mpsc::channel();
        let caller = std::thread::spawn(move || {
            let client = SharedContextClient::new(IpcContextClient::new(worker));
            let result =
                requests
                    .into_iter()
                    .try_for_each(|request| match client.request(request) {
                        Ok(ContextServiceResponse::NativeWindowInventoryPublished) => Ok(()),
                        other => Err(format!("unexpected inventory response: {other:?}")),
                    });
            let _ = finished.send(result);
        });
        let result = result.recv_timeout(Duration::from_secs(5));
        // Cleanup precedes assertions so a regressed protocol cannot hang libtest.
        drop(shutdown);
        caller.join().unwrap();
        server.join().unwrap().unwrap();
        result.expect("inventory response deadline").unwrap();
        assert_eq!(*service.0.lock().unwrap(), expected);
    }
    #[test]
    fn malformed_envelope_closes_channel_without_an_invented_correlation() {
        let (mut worker, parent) = UnixStream::pair().unwrap();
        worker
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let shutdown = ShutdownOnDrop(parent.try_clone().unwrap());
        let service = Arc::new(Recording::default());
        let served = service.clone();
        let server = std::thread::spawn(move || serve_context_transport(parent, served));
        let malformed = format!(
            "{{\"protocol_version\":{CONTEXT_SERVICE_PROTOCOL_VERSION},\"route\":null,\"message_id\":17,\"request\":null}}\n"
        );
        worker.write_all(malformed.as_bytes()).unwrap();
        let mut byte = [0];
        let received = worker.read(&mut byte);
        drop(shutdown);
        let result = server.join().unwrap();
        assert_eq!(
            received.unwrap(),
            0,
            "malformed request must not receive correlation zero"
        );
        assert!(
            matches!(result, Err(RuntimeError::ProtocolViolation(message)) if message.contains("malformed context request"))
        );
        assert!(service.0.lock().unwrap().is_empty());
    }
}
