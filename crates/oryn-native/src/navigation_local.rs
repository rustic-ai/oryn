//! Native local-response grants are reserved before IO and retained through
//! actor commit. A dropped or late result queues token cleanup with ACK custody.
use super::*;
use std::task::Poll;

pub(super) struct PreparedLocal {
    pub token: String,
    pub bytes: Vec<u8>,
    pub media_type: String,
    pub headers: Vec<(String, String)>,
    pub base_url: Option<String>,
    _lease: crate::context_service::ContextLocalNavigationLease,
}
pub(super) struct PendingLocal {
    receiver: Option<oneshot::Receiver<Result<PreparedLocal, RuntimeError>>>,
    deadline: Instant,
}
impl PendingLocal {
    pub fn start(
        client: SharedContextClient,
        page_id: PageId,
        request: crate::context_service::ContextServiceRequest,
        deadline: Instant,
    ) -> Result<Self, RuntimeError> {
        use crate::context_service::{
            ContextServiceRequest as Request, ContextServiceResponse as Response,
        };
        let (blob, blank) = match &request {
            Request::PrepareBlankNavigation { .. }
            | Request::PrepareJavascriptNavigation { .. } => (false, true),
            Request::PrepareBlobNavigation { .. } => (true, false),
            Request::PrepareLocalDataNavigation { .. } => (false, false),
            Request::PrepareInitialLocalNavigation { blob, .. }
            | Request::PrepareServiceLocalNavigation { blob, .. } => (*blob, false),
            _ => {
                return Err(RuntimeError::ProtocolViolation(
                    "invalid local navigation preparation request".into(),
                ));
            }
        };
        let lease = client.reserve_local_navigation(page_id)?;
        let job = client.track_resource_job()?;
        let (sender, receiver) = oneshot::channel();
        crate::executable::resource_io_executor::submit(move || {
            let _job = job;
            if sender.is_closed() {
                return;
            }
            let result = (|| {
                if Instant::now() >= deadline {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: "navigation wall time".into(),
                    });
                }
                let response = client.request(request)?;
                match &response {
                    Response::BlobNavigationPrepared { token, .. }
                    | Response::LocalDataNavigationPrepared { token, .. }
                    | Response::BlankNavigationPrepared { token, .. } => lease.attach(token),
                    _ => {}
                }
                let (token, encoded, media_type, headers, base_url) = match response {
                    Response::BlobNavigationPrepared {
                        token,
                        body_base64,
                        media_type,
                        headers,
                        ..
                    } if blob && !blank => (token, body_base64, media_type, headers, None),
                    Response::LocalDataNavigationPrepared {
                        token,
                        body_base64,
                        media_type,
                        headers,
                        ..
                    } if !blob && !blank => (token, body_base64, media_type, headers, None),
                    Response::BlankNavigationPrepared {
                        token,
                        body_base64,
                        media_type,
                        headers,
                        base_url,
                        ..
                    } if blank => (token, body_base64, media_type, headers, Some(base_url)),
                    _ => {
                        return Err(RuntimeError::ProtocolViolation(
                            "invalid local navigation preparation response".into(),
                        ));
                    }
                };
                if encoded.len() > 12 * 1024 * 1024 {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: "local navigation encoded bytes".into(),
                    });
                }
                let bytes = BASE64.decode(encoded).map_err(|error| {
                    RuntimeError::ProtocolViolation(format!(
                        "invalid local navigation bytes: {error}"
                    ))
                })?;
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err(RuntimeError::WorkerLimitExceeded {
                        resource: "local navigation bytes".into(),
                    });
                }
                Ok(PreparedLocal {
                    token,
                    bytes,
                    media_type,
                    headers,
                    base_url,
                    _lease: lease,
                })
            })();
            let _ = sender.send(result);
        })
        .map_err(|error| {
            RuntimeError::Network(format!("local navigation IO admission: {error}"))
        })?;
        Ok(Self {
            receiver: Some(receiver),
            deadline,
        })
    }
    pub fn poll(&mut self) -> Poll<Result<PreparedLocal, RuntimeError>> {
        if Instant::now() >= self.deadline {
            self.receiver.take();
            return Poll::Ready(Err(RuntimeError::WorkerLimitExceeded {
                resource: "navigation wall time".into(),
            }));
        }
        let Some(receiver) = self.receiver.as_mut() else {
            return Poll::Ready(Err(RuntimeError::NavigationCancelled));
        };
        match receiver.try_recv() {
            Ok(result) => {
                self.receiver.take();
                Poll::Ready(result)
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => Poll::Pending,
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.receiver.take();
                Poll::Ready(Err(RuntimeError::Network(
                    "local navigation preparation ended without a response".into(),
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_service::{
        CONTEXT_SERVICE_PROTOCOL_VERSION, ContextServiceRequest as Request,
        ContextServiceResponse as Response, IpcContextClient,
    };
    use std::io::{BufRead, BufReader, Write};

    #[test]
    fn abandoned_local_preparation_discards_late_grant_after_delivery_receipt() {
        let (local, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (entered, started) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let mut reader = BufReader::new(peer.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            let response = serde_json::json!({"protocol_version":CONTEXT_SERVICE_PROTOCOL_VERSION,"route":null,
                "correlation_id":request["message_id"],"response":Response::LocalDataNavigationPrepared{
                    token:"late-local-grant".into(),body_base64:"PGI+aGVsbG88L2I+".into(),media_type:"text/html".into(),headers:Vec::new(),serialized:"null".into(),opaque_key:Some("local-origin".into())},"error":null});
            writeln!(peer, "{response}").unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            let receipt: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(receipt["accepted"], true);
            line.clear();
            reader.read_line(&mut line).unwrap();
            let cleanup: serde_json::Value = serde_json::from_str(&line).unwrap();
            let request: Request = serde_json::from_value(cleanup["request"].clone()).unwrap();
            assert!(
                matches!(request,Request::DiscardPreparedLocalNavigation{page_id:PageId(7),token} if token=="late-local-grant")
            );
            let response = serde_json::json!({"protocol_version":CONTEXT_SERVICE_PROTOCOL_VERSION,"route":null,
                "correlation_id":cleanup["message_id"],"response":Response::LocalNavigationDiscarded,"error":null});
            writeln!(peer, "{response}").unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            let receipt: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(receipt["accepted"], true);
        });
        let client = SharedContextClient::new(IpcContextClient::new(local));
        let pending = PendingLocal::start(
            client.clone(),
            PageId(7),
            Request::PrepareInitialLocalNavigation {
                page_id: PageId(7),
                url: "data:text/html,%3Cb%3Ehello%3C/b%3E".into(),
                blob: false,
            },
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        started.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(pending);
        assert!(client.retire_resource_jobs(Duration::ZERO).is_err());
        release.send(()).unwrap();
        server.join().unwrap();
        client
            .drain_resource_teardown(Duration::from_secs(2))
            .unwrap();
    }
}
