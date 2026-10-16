//! Staged native gate shared by generic and owned IDB admission queues.
//! Request identity ordering is per transaction, never database-name matching.
use super::*;

pub(super) fn earlier_unsubmitted_on_transaction(state: &V8BindingState, request_id: u64) -> bool {
    let transaction_id = {
        let objects = state.platform_objects.borrow();
        match objects.get(&request_id) {
            Some(PlatformObjectRecord::IdbRequest(request)) => request.transaction_id,
            _ => None,
        }
    };
    let Some(transaction_id) = transaction_id else { return false };
    let objects = state.platform_objects.borrow();
    let Some(PlatformObjectRecord::IdbTransaction(transaction)) = objects.get(&transaction_id) else { return false };
    let owned = state.queued_owned_idb.borrow();
    let ordinary = state.queued_idb_operations.borrow();
    transaction.request_queue.iter().copied().take_while(|id| *id != request_id).any(|id| {
        owned.iter().any(|operation| operation.request_id == id)
            || ordinary.iter().any(|operation| matches!(&operation.pending,
                PendingPlatformOperation::IndexedDb { request_id, .. } if *request_id == id))
    })
}
