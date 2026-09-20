//! Off-actor terminal publication. Capacity is reserved before author dispatch;
//! a lost acknowledgement retries the identical receipt and never replays JS.
use crate::{
    context_service::{ContextServiceRequest, ContextServiceResponse, SharedContextClient},
    runtime::RuntimeError,
};
#[cfg(feature = "v8-host")]
use oryn_js::navigation::NavigationExecutionObserver;
use oryn_js::navigation::{NavigationExecutionOutcome, NavigationExecutionToken};
use std::sync::{Arc, Mutex};
#[cfg(feature = "v8-host")]
use std::sync::{
    Condvar,
    atomic::{AtomicUsize, Ordering},
};
#[cfg(feature = "v8-host")]
static LIVE: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "v8-host")]
enum Publication {
    Admitted,
    Terminal(NavigationExecutionOutcome),
}
#[cfg(feature = "v8-host")]
struct Reservation;
#[cfg(feature = "v8-host")]
impl Drop for Reservation {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(feature = "v8-host")]
pub(crate) fn observer(
    client: SharedContextClient,
    page_id: oryn_common::v3::PageId,
    receipt: NavigationExecutionToken,
) -> Result<NavigationExecutionObserver, RuntimeError> {
    if receipt.0.len() != 32 {
        return Err(RuntimeError::ProtocolViolation(
            "invalid native navigation receipt".into(),
        ));
    }
    LIVE.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
        (count < 256).then_some(count + 1)
    })
    .map_err(|_| {
        RuntimeError::ProtocolViolation("navigation publication capacity exhausted".into())
    })?;
    let reservation = Reservation;
    let publication = Arc::new((
        Mutex::new(std::collections::VecDeque::<Publication>::new()),
        Condvar::new(),
    ));
    let worker = publication.clone();
    let worker_client = client.clone();
    let worker_receipt = receipt.clone();
    std::thread::Builder::new()
        .name("oryn-navigation-settlement".into())
        .spawn(move || {
            let _reservation = reservation;
            let client = worker_client;
            let receipt = worker_receipt;
            let (lock, changed) = &*worker;
            let mut claimed = false;
            loop {
                let mut pending = lock.lock().unwrap_or_else(|error| error.into_inner());
                while pending.is_empty() {
                    pending = changed
                        .wait(pending)
                        .unwrap_or_else(|error| error.into_inner());
                }
                let publication = pending.pop_front().expect("native publication");
                drop(pending);
                let mut retry = std::time::Duration::from_millis(25);
                loop {
                    let published = match &publication {
                        Publication::Admitted => publish_admission_once(&client, page_id, &receipt),
                        Publication::Terminal(outcome) => {
                            publish_once(&client, page_id, &receipt, outcome, &mut claimed)
                        }
                    };
                    if published {
                        break;
                    }
                    std::thread::sleep(retry);
                    retry = (retry * 2).min(std::time::Duration::from_secs(1));
                }
                if matches!(publication, Publication::Terminal(_)) {
                    break;
                }
            }
        })
        .map_err(|error| {
            RuntimeError::ProtocolViolation(format!(
                "navigation settlement worker unavailable: {error}"
            ))
        })?;
    let admission = publication.clone();
    let observer = NavigationExecutionObserver::with_admission(
        move || {
            let (lock, changed) = &*admission;
            lock.lock()
                .unwrap_or_else(|error| error.into_inner())
                .push_back(Publication::Admitted);
            changed.notify_one();
        },
        move |outcome| {
            let (lock, changed) = &*publication;
            lock.lock()
                .unwrap_or_else(|error| error.into_inner())
                .push_back(Publication::Terminal(outcome));
            changed.notify_one();
        },
    );
    match client.request(ContextServiceRequest::ClaimNavigationExecution { page_id, receipt }) {
        Ok(ContextServiceResponse::NavigationExecutionClaimed) => Ok(observer),
        Ok(ContextServiceResponse::NavigationExecutionClaimRejected) => {
            Err(RuntimeError::NavigationCancelled)
        }
        Ok(_) => Err(RuntimeError::ProtocolViolation(
            "invalid native navigation claim response".into(),
        )),
        Err(error) => Err(error),
    }
}

fn publish_admission_once(
    client: &SharedContextClient,
    page_id: oryn_common::v3::PageId,
    receipt: &NavigationExecutionToken,
) -> bool {
    matches!(
        client.request(ContextServiceRequest::AcceptNavigationExecution {
            page_id,
            receipt: receipt.clone()
        }),
        Ok(ContextServiceResponse::NavigationExecutionAccepted
            | ContextServiceResponse::NavigationExecutionRetired)
            | Err(RuntimeError::Closed)
    )
}

fn publish_once(
    client: &SharedContextClient,
    page_id: oryn_common::v3::PageId,
    receipt: &NavigationExecutionToken,
    outcome: &NavigationExecutionOutcome,
    claimed: &mut bool,
) -> bool {
    if !*claimed {
        match client.request(ContextServiceRequest::ClaimNavigationExecution {
            page_id,
            receipt: receipt.clone(),
        }) {
            Ok(ContextServiceResponse::NavigationExecutionClaimed) => *claimed = true,
            Ok(ContextServiceResponse::NavigationExecutionClaimRejected)
            | Err(RuntimeError::Closed) => return true,
            _ => return false,
        }
    }
    match client.request(ContextServiceRequest::SettleNavigationExecution {
        page_id,
        receipt: receipt.clone(),
        outcome: outcome.clone(),
    }) {
        Ok(
            ContextServiceResponse::NavigationExecutionSettled
            | ContextServiceResponse::NavigationExecutionRetired,
        )
        | Err(RuntimeError::Closed) => true,
        _ => {
            // Recheck live custody after an uncertain ACK. Rejected claims
            // dispose a settled/pruned receipt; they do not accept our result.
            // Parent evidence still exposes pruning and the original outcome.
            *claimed = false;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_service::ContextTransport;
    struct Transport {
        responses: Mutex<std::collections::VecDeque<Result<ContextServiceResponse, RuntimeError>>>,
        requests: Mutex<Vec<ContextServiceRequest>>,
    }
    impl ContextTransport for Transport {
        fn request_routed(
            &self,
            _route: Option<&crate::worker_route::WorkerRoute>,
            request: ContextServiceRequest,
        ) -> Result<ContextServiceResponse, RuntimeError> {
            self.request(request)
        }
        fn request(
            &self,
            request: ContextServiceRequest,
        ) -> Result<ContextServiceResponse, RuntimeError> {
            self.requests.lock().unwrap().push(request);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected replay")
        }
    }
    fn client(
        responses: Vec<Result<ContextServiceResponse, RuntimeError>>,
    ) -> (SharedContextClient, Arc<Transport>) {
        let transport = Arc::new(Transport {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
        });
        (
            SharedContextClient::for_test_transport(
                transport.clone(),
                crate::worker_route::WorkerRoute::issue(1, 7, 1).unwrap(),
            ),
            transport,
        )
    }
    #[test]
    fn admission_ack_retry_preserves_exact_receipt_without_terminal_publication() {
        let (client, transport) = client(vec![
            Err(RuntimeError::ContextService("lost admission ack".into())),
            Ok(ContextServiceResponse::NavigationExecutionAccepted),
        ]);
        let token = NavigationExecutionToken(vec![3; 32]);
        assert!(!publish_admission_once(
            &client,
            oryn_common::v3::PageId(7),
            &token
        ));
        assert!(publish_admission_once(
            &client,
            oryn_common::v3::PageId(7),
            &token
        ));
        let requests = transport.requests.lock().unwrap();
        assert_eq!(
            serde_json::to_value(&requests[0]).unwrap(),
            serde_json::to_value(&requests[1]).unwrap()
        );
        assert!(requests.iter().all(|request| matches!(
            request,
            ContextServiceRequest::AcceptNavigationExecution { .. }
        )));
    }
    #[test]
    fn lost_ack_rechecks_custody_and_retries_same_terminal_without_callbacks() {
        let (client, transport) = client(vec![
            Ok(ContextServiceResponse::NavigationExecutionClaimed),
            Err(RuntimeError::ContextService("lost acknowledgement".into())),
            Ok(ContextServiceResponse::NavigationExecutionClaimed),
            Ok(ContextServiceResponse::NavigationExecutionSettled),
        ]);
        let token = NavigationExecutionToken(vec![7; 32]);
        let outcome = NavigationExecutionOutcome::Committed {
            document_generation: 42,
        };
        let mut claimed = false;
        assert!(!publish_once(
            &client,
            oryn_common::v3::PageId(7),
            &token,
            &outcome,
            &mut claimed
        ));
        assert!(!claimed);
        assert!(publish_once(
            &client,
            oryn_common::v3::PageId(7),
            &token,
            &outcome,
            &mut claimed
        ));
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(
            serde_json::to_value(&requests[1]).unwrap(),
            serde_json::to_value(&requests[3]).unwrap()
        );
    }
    #[test]
    fn unknown_claim_retains_custody_and_terminal_rejection_sends_no_outcome() {
        let (client, transport) = client(vec![
            Err(RuntimeError::ContextService("claim ack missing".into())),
            Ok(ContextServiceResponse::NavigationExecutionClaimRejected),
        ]);
        let token = NavigationExecutionToken(vec![8; 32]);
        let outcome = NavigationExecutionOutcome::Failed {
            message: "never dispatched".into(),
        };
        let mut claimed = false;
        assert!(!publish_once(
            &client,
            oryn_common::v3::PageId(7),
            &token,
            &outcome,
            &mut claimed
        ));
        assert!(!claimed);
        assert!(publish_once(
            &client,
            oryn_common::v3::PageId(7),
            &token,
            &outcome,
            &mut claimed
        ));
        assert!(
            transport
                .requests
                .lock()
                .unwrap()
                .iter()
                .all(|request| matches!(
                    request,
                    ContextServiceRequest::ClaimNavigationExecution { .. }
                ))
        );
    }
    #[test]
    fn settled_or_pruned_receipt_disposes_after_uncertain_ack_without_republishing() {
        let (client, transport) = client(vec![
            Ok(ContextServiceResponse::NavigationExecutionClaimed),
            Err(RuntimeError::ContextService("settled but ack lost".into())),
            Ok(ContextServiceResponse::NavigationExecutionClaimRejected),
        ]);
        let token = NavigationExecutionToken(vec![9; 32]);
        let mut claimed = false;
        assert!(!publish_once(
            &client,
            oryn_common::v3::PageId(7),
            &token,
            &NavigationExecutionOutcome::Completed,
            &mut claimed
        ));
        assert!(publish_once(
            &client,
            oryn_common::v3::PageId(7),
            &token,
            &NavigationExecutionOutcome::Completed,
            &mut claimed
        ));
        assert_eq!(
            transport
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|request| matches!(
                    request,
                    ContextServiceRequest::SettleNavigationExecution { .. }
                ))
                .count(),
            1
        );
    }
}
