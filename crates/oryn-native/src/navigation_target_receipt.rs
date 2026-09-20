//! Immutable parent-selected navigable identity. A receipt is not permission
//! to load a URL; final document-source policy must be checked independently.
use super::*;
use crate::host::webdriver_script::AutomationRealmSnapshot;
use oryn_js::navigation::NavigationTargetReceipt;
use sha2::{Digest, Sha256};

const MAX_RECEIPT: usize = 262_144;

#[derive(serde::Serialize, serde::Deserialize)]
struct Selection {
    version: u8,
    context: u64,
    source: PageId,
    destination: PageId,
    frame: Option<AutomationRealmSnapshot>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Sealed {
    payload: Vec<u8>,
    mac: [u8; 32],
}

fn authenticate(key: &[u8; 32], payload: &[u8]) -> [u8; 32] {
    let mut inner = [0x36u8; 64];
    let mut outer = [0x5cu8; 64];
    for (index, byte) in key.iter().enumerate() {
        inner[index] ^= byte;
        outer[index] ^= byte;
    }
    let mut hash = Sha256::new();
    hash.update(inner);
    hash.update(b"oryn.navigation-target.v1\0");
    hash.update(payload);
    let digest = hash.finalize();
    let mut hash = Sha256::new();
    hash.update(outer);
    hash.update(digest);
    hash.finalize().into()
}

impl BrowserContext {
    fn navigation_target_key(&self) -> Result<&[u8; 32], RuntimeError> {
        self.inner
            .form_navigation_key
            .get_or_init(|| {
                let mut key = [0; 32];
                getrandom::fill(&mut key).map_err(|error| error.to_string())?;
                Ok(key)
            })
            .as_ref()
            .map_err(|error| RuntimeError::ContextService(error.clone()))
    }

    pub(crate) fn capture_navigation_target(
        &self,
        source: PageId,
        creator: &oryn_js::worker_wire::Creator,
        destination: PageId,
        frame: Option<AutomationRealmSnapshot>,
    ) -> Result<NavigationTargetReceipt, RuntimeError> {
        let oryn_js::worker_wire::Creator::Window {
            target,
            document_generation,
            stream_epoch,
        } = creator
        else {
            return Err(RuntimeError::ProtocolViolation(
                "navigation selection requires a Window".into(),
            ));
        };
        self.window_worker_creator(source, target, *document_generation, *stream_epoch)?;
        self.seal_navigation_target(source, destination, frame)
    }

    pub(crate) fn capture_document_navigation_target(
        &self,
        source: PageId,
        document: &oryn_dom::DocumentProvenance,
        destination: PageId,
        frame: Option<AutomationRealmSnapshot>,
    ) -> Result<NavigationTargetReceipt, RuntimeError> {
        self.document_popup_source(source, document)?;
        self.seal_navigation_target(source, destination, frame)
    }

    fn seal_navigation_target(
        &self,
        source: PageId,
        destination: PageId,
        frame: Option<AutomationRealmSnapshot>,
    ) -> Result<NavigationTargetReceipt, RuntimeError> {
        let live = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| {
                RuntimeError::ContextService("navigation target registry poisoned".into())
            })?
            .get(&destination)
            .is_some_and(|record| record.is_live());
        if !live {
            return Err(RuntimeError::PageNotFound(destination));
        }
        if let Some(frame) = &frame {
            if !frame.same_navigable_as(frame) || frame.parent_realm.is_none() {
                return Err(RuntimeError::ProtocolViolation(
                    "incomplete selected frame identity".into(),
                ));
            }
            self.window_worker_creator(
                destination,
                &oryn_js::message_wire::WindowTarget::Frame(frame.frame_token.clone().unwrap()),
                frame.document_generation,
                frame.document_authority_epoch,
            )?;
        }
        let payload = serde_json::to_vec(&Selection {
            version: 1,
            context: self.inner.id,
            source,
            destination,
            frame,
        })
        .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        if payload.len() > MAX_RECEIPT / 4 {
            return Err(RuntimeError::ProtocolViolation(
                "navigation target receipt capacity".into(),
            ));
        }
        let sealed = Sealed {
            mac: authenticate(self.navigation_target_key()?, &payload),
            payload,
        };
        let bytes = serde_json::to_vec(&sealed)
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        if bytes.len() > MAX_RECEIPT {
            return Err(RuntimeError::ProtocolViolation(
                "navigation target receipt capacity".into(),
            ));
        }
        Ok(NavigationTargetReceipt(bytes))
    }

    pub(crate) fn read_navigation_target(
        &self,
        source: PageId,
        receipt: &NavigationTargetReceipt,
    ) -> Result<(PageId, Option<AutomationRealmSnapshot>), RuntimeError> {
        let invalid =
            || RuntimeError::ProtocolViolation("invalid navigation target receipt".into());
        if receipt.0.len() > MAX_RECEIPT {
            return Err(invalid());
        }
        let sealed: Sealed = serde_json::from_slice(&receipt.0).map_err(|_| invalid())?;
        let expected = authenticate(self.navigation_target_key()?, &sealed.payload);
        let difference = expected
            .iter()
            .zip(sealed.mac)
            .fold(0u8, |diff, (left, right)| diff | (*left ^ right));
        if difference != 0 {
            return Err(invalid());
        }
        let selection: Selection =
            serde_json::from_slice(&sealed.payload).map_err(|_| invalid())?;
        if selection.version != 1
            || selection.context != self.inner.id
            || selection.source != source
        {
            return Err(invalid());
        }
        let live = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| {
                RuntimeError::ContextService("navigation target registry poisoned".into())
            })?
            .get(&selection.destination)
            .is_some_and(|record| record.is_live());
        if !live {
            return Err(RuntimeError::PageNotFound(selection.destination));
        }
        // Frame connection liveness is resolved on its owning worker. Requiring
        // the captured document generation here would reject a valid commit in
        // the same selected frame. No name lookup is performed at consumption.
        Ok((selection.destination, selection.frame))
    }

    /// Resolve only the captured connection. Names and document generations can
    /// change after selection; neither may redirect the operation to a new frame.
    #[cfg(test)]
    pub(crate) fn resolve_navigation_target(
        &self,
        source: PageId,
        receipt: &NavigationTargetReceipt,
    ) -> Result<(PageId, Option<AutomationRealmSnapshot>), RuntimeError> {
        self.resolve_navigation_target_with_local(source, receipt, &BTreeMap::new())
    }

    pub(crate) fn resolve_navigation_target_with_local(
        &self,
        source: PageId,
        receipt: &NavigationTargetReceipt,
        local: &BTreeMap<PageId, Vec<(String, AutomationRealmSnapshot)>>,
    ) -> Result<(PageId, Option<AutomationRealmSnapshot>), RuntimeError> {
        use crate::host::webdriver_script::{FrameCommand, FrameResponse};
        let (destination, selected) = self.read_navigation_target(source, receipt)?;
        let Some(selected) = selected else {
            return Ok((destination, None));
        };
        if let Some(inventory) = local.get(&destination) {
            let current = inventory
                .iter()
                .map(|(_, snapshot)| snapshot)
                .find(|snapshot| snapshot.same_navigable_as(&selected))
                .ok_or_else(|| {
                    RuntimeError::ContextService("selected local navigation frame retired".into())
                })?;
            let token = current.frame_token.clone().ok_or_else(|| {
                RuntimeError::ProtocolViolation("selected local frame token missing".into())
            })?;
            self.window_worker_creator(
                destination,
                &oryn_js::message_wire::WindowTarget::Frame(token),
                current.document_generation,
                current.document_authority_epoch,
            )?;
            return Ok((destination, Some(current.clone())));
        }
        // Local frame selection stays on its owning native thread. A context
        // RPC must never reenter the blocked source worker to inspect its tree.
        if destination == source {
            return Err(RuntimeError::ProtocolViolation(
                "local selected frame requires native thread resolution".into(),
            ));
        }
        let handle = self
            .inner
            .group
            .pages
            .lock()
            .map_err(|_| {
                RuntimeError::ContextService("navigation target registry poisoned".into())
            })?
            .get(&destination)
            .filter(|record| record.is_live())
            .map(|record| record.handle.clone())
            .ok_or(RuntimeError::PageNotFound(destination))?;
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let response = executor
            .block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(5),
                    handle.webdriver_frames(FrameCommand::NamedInventory),
                )
                .await
            })
            .map_err(|_| {
                RuntimeError::ContextService("selected target inventory deadline exceeded".into())
            })?
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
        let FrameResponse::NamedInventory(inventory) = response else {
            return Err(RuntimeError::ProtocolViolation(
                "invalid selected target inventory".into(),
            ));
        };
        let current = inventory
            .into_iter()
            .map(|(_, snapshot)| snapshot)
            .find(|snapshot| snapshot.same_navigable_as(&selected))
            .ok_or_else(|| {
                RuntimeError::ContextService("selected navigation frame retired".into())
            })?;
        Ok((destination, Some(current)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_binds_context_source_and_destination_without_exposing_authority() {
        use oryn_js::{
            message_wire::{MessageRequest, WindowTarget},
            worker_wire::Creator,
        };
        let context = Browser::new().new_context();
        let source = context.new_page();
        let destination = context.new_popup(source.page_id(), None);
        context
            .install_document_security(source.page_id(), "https://source.test/".into(), &[])
            .unwrap();
        context
            .handle_message_wire(
                source.page_id(),
                MessageRequest::RegisterRootDocument {
                    document_generation: 1,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let receipt = context
            .capture_navigation_target(source.page_id(), &creator, destination.page_id(), None)
            .unwrap();
        assert_eq!(
            context
                .read_navigation_target(source.page_id(), &receipt)
                .unwrap()
                .0,
            destination.page_id()
        );
        assert_eq!(
            context
                .resolve_navigation_target(source.page_id(), &receipt)
                .unwrap()
                .0,
            destination.page_id()
        );
        assert!(
            context
                .read_navigation_target(destination.page_id(), &receipt)
                .is_err()
        );
        assert!(
            Browser::new()
                .new_context()
                .read_navigation_target(source.page_id(), &receipt)
                .is_err()
        );
        let mut damaged = receipt.clone();
        let middle = damaged.0.len() / 2;
        damaged.0[middle] ^= 1;
        assert!(
            context
                .read_navigation_target(source.page_id(), &damaged)
                .is_err()
        );
        let mut envelope: Sealed = serde_json::from_slice(&receipt.0).unwrap();
        let mut selection: Selection = serde_json::from_slice(&envelope.payload).unwrap();
        selection.destination = source.page_id();
        envelope.payload = serde_json::to_vec(&selection).unwrap();
        let redirected = NavigationTargetReceipt(serde_json::to_vec(&envelope).unwrap());
        assert!(
            context
                .read_navigation_target(source.page_id(), &redirected)
                .is_err(),
            "editing destination cannot preserve receipt integrity"
        );
        assert!(
            context
                .read_navigation_target(
                    source.page_id(),
                    &NavigationTargetReceipt(vec![0; MAX_RECEIPT + 1])
                )
                .is_err()
        );
        assert!(
            context
                .form_fetch_source(
                    &oryn_js::navigation::FormNavigationAuthority(receipt.0.clone()),
                    source.page_id()
                )
                .is_err()
        );
        assert!(!format!("{receipt:?}").contains("source.test"));
    }
}
