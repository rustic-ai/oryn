//! A prepared frame response becomes readable only after its native policy
//! ticket binds the new child Document. IO runs on owned bounded resource lanes.
use super::*;
use crate::context_service::{
    ContextFrameResponseLease, ContextServiceRequest, ContextServiceResponse, SharedContextClient,
};
use std::sync::Arc;
#[derive(Clone)]
pub(super) struct FrameResponseBody {
    pub(super) client: SharedContextClient,
    pub(super) page_id: oryn_common::v3::PageId,
    pub(super) id: u64,
    pub(super) lease: Arc<ContextFrameResponseLease>,
}
impl std::fmt::Debug for FrameResponseBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameResponseBody")
            .field("id", &self.id)
            .finish()
    }
}
impl FrameResponseBody {
    pub(super) fn decoded(
        self,
        creator: oryn_js::worker_wire::Creator,
        content_type: Option<String>,
        fallback: Option<&str>,
        budget: &Arc<std::sync::atomic::AtomicUsize>,
        timing: Option<oryn_js::navigation_timing::NavigationTiming>,
    ) -> Result<crate::host::DocumentStream, HostError> {
        let reservation =
            ScriptSourceReservation::reserve(budget, 264 * 1024).ok_or_else(|| {
                HostError::LimitExceeded {
                    resource: "frame response decoder custody".into(),
                }
            })?;
        let encoding = Arc::new(std::sync::Mutex::new(None));
        let input = DecodedFrameInput {
            timing,
            received_bytes: 0,
            replay: Default::default(),
            decoder: frame_text_decoder(content_type.as_deref(), fallback),
            body: self,
            creator,
            pending: None,
            encoding: encoding.clone(),
            done: false,
            _bytes: reservation,
        };
        Ok(crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(input),
            content_type,
            encoding,
        })
    }
}
fn frame_text_decoder(
    content_type: Option<&str>,
    inherited: Option<&str>,
) -> oryn_net::IncrementalTextDecoder {
    oryn_net::IncrementalTextDecoder::with_fallback(
        content_type,
        crate::network::navigation_text_kind(content_type),
        inherited,
    )
    .with_html_detection()
}
struct DecodedFrameInput {
    timing: Option<oryn_js::navigation_timing::NavigationTiming>,
    received_bytes: u64,
    replay: crate::html_encoding_replay::HtmlEncodingReplay,
    body: FrameResponseBody,
    creator: oryn_js::worker_wire::Creator,
    pending: Option<std::sync::mpsc::Receiver<Result<(Vec<u8>, bool, Option<u64>), String>>>,
    decoder: oryn_net::IncrementalTextDecoder,
    encoding: Arc<std::sync::Mutex<Option<&'static str>>>,
    done: bool,
    _bytes: ScriptSourceReservation,
}
impl Drop for DecodedFrameInput {
    fn drop(&mut self) {
        self.body.lease.cancel();
    }
}
impl crate::host::DocumentInput for DecodedFrameInput {
    fn navigation_timing(&self) -> Option<oryn_js::navigation_timing::NavigationTiming> {
        self.timing.clone()
    }
    fn encoding_confidence(&self) -> oryn_net::HtmlEncodingConfidence {
        self.decoder.html_encoding_confidence()
    }
    fn commit_encoding(&mut self) {
        self.replay.commit(&mut self.decoder)
    }
    fn encoding_restart_needed(&self, label: &str) -> bool {
        self.decoder.html_encoding_restart_needed(label)
    }
    fn restart_encoding(&mut self, label: &str) -> Result<Option<String>, HostError> {
        let text = self.replay.restart(&mut self.decoder, label);
        *self.encoding.lock().expect("frame encoding") = self.decoder.encoding_name();
        Ok(text)
    }
    fn poll_next(&mut self) -> std::task::Poll<Option<Result<String, HostError>>> {
        use std::task::Poll;
        if self.done {
            return Poll::Ready(None);
        }
        if let Some(receiver) = self.pending.as_ref() {
            match receiver.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    match result {
                        Ok((bytes, done, encoded_body_size)) => {
                            self.received_bytes =
                                self.received_bytes.saturating_add(bytes.len() as u64);
                            let mut output = match self.replay.push(&mut self.decoder, &bytes) {
                                Ok(text) => text,
                                Err(error) => {
                                    self.done = true;
                                    self.body.lease.cancel();
                                    return Poll::Ready(Some(Err(error)));
                                }
                            };
                            if done {
                                output.push_str(&self.replay.finish(&mut self.decoder));
                                self.done = true;
                                if let Some(timing) = &self.timing {
                                    timing.finish_encoded(
                                        std::time::Instant::now(),
                                        self.received_bytes,
                                        encoded_body_size,
                                    );
                                }
                                self.body.lease.finish();
                            }
                            *self.encoding.lock().expect("frame encoding") =
                                self.decoder.encoding_name();
                            return Poll::Ready(Some(Ok(output)));
                        }
                        Err(error) => {
                            self.done = true;
                            self.body.lease.cancel();
                            return Poll::Ready(Some(Err(HostError::Execution(error))));
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return Poll::Pending,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.done = true;
                    self.body.lease.cancel();
                    return Poll::Ready(Some(Err(HostError::Execution(
                        "frame response reader disconnected".into(),
                    ))));
                }
            }
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let body = self.body.clone();
        let creator = self.creator.clone();
        let ownership = match body.client.track_resource_job() {
            Ok(owner) => owner,
            Err(error) => {
                self.done = true;
                self.body.lease.cancel();
                return Poll::Ready(Some(Err(HostError::Execution(error.to_string()))));
            }
        };
        match resource_io_executor::submit(move || {
            let _ownership = ownership;
            let result = body
                .client
                .request(ContextServiceRequest::HttpStreamRead {
                    page_id: body.page_id,
                    stream_id: body.id,
                    max_bytes: 64 * 1024,
                    creator: Some(creator),
                })
                .map_err(|error| error.to_string())
                .and_then(|response| match response {
                    ContextServiceResponse::HttpStreamChunk {
                        encoded_body_size,
                        stream_id,
                        body_base64,
                        done,
                    } if stream_id == body.id => decode_chunk(&body_base64, done)
                        .map(|(bytes, done)| (bytes, done, encoded_body_size)),
                    _ => Err("invalid frame response chunk".into()),
                });
            let _ = sender.send(result);
        }) {
            Ok(()) => {
                self.pending = Some(receiver);
                Poll::Pending
            }
            Err(error) => {
                self.done = true;
                self.body.lease.cancel();
                Poll::Ready(Some(Err(HostError::Execution(error))))
            }
        }
    }
}

fn decode_chunk(encoded: &str, done: bool) -> Result<(Vec<u8>, bool), String> {
    if encoded.len() > ((64 * 1024 + 2) / 3) * 4 {
        return Err("frame response chunk exceeds encoded custody".into());
    }
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .map_err(|error| error.to_string())?;
    if bytes.len() > 64 * 1024 {
        return Err("frame response chunk exceeds decoded custody".into());
    }
    if bytes.is_empty() && !done {
        return Err("nonterminal frame response chunk is empty".into());
    }
    Ok((bytes, done))
}
#[cfg(test)]
mod tests {
    #[test]
    fn selected_default_is_not_an_inherited_charset() {
        let bytes = "<p>Это тест кодировки символов. 日本語</p>".as_bytes();
        let mut detected = frame_text_decoder(Some("text/html"), None);
        let mut text = detected.push(bytes);
        text.push_str(&detected.finish());
        assert_eq!(text, std::str::from_utf8(bytes).unwrap());
        assert_eq!(detected.encoding_name(), Some("UTF-8"));
        let mut inherited = frame_text_decoder(Some("text/html"), Some("windows-1251"));
        let mut text = inherited.push(bytes);
        text.push_str(&inherited.finish());
        assert_eq!(inherited.encoding_name(), Some("windows-1251"));
        assert_ne!(text, std::str::from_utf8(bytes).unwrap());
        let mut header = frame_text_decoder(Some("text/html;charset=UTF-8"), Some("windows-1251"));
        let mut text = header.push(bytes);
        text.push_str(&header.finish());
        assert_eq!(text, std::str::from_utf8(bytes).unwrap());
        assert_eq!(header.encoding_name(), Some("UTF-8"));
    }
    use super::*;
    use std::sync::Arc;
    #[test]
    fn frame_decoder_retains_network_activity_between_chunk_polls() {
        let context = crate::runtime::Browser::new().new_context();
        let page = context.new_page();
        let client = SharedContextClient::direct(context.clone(), page.page_id());
        let lease = client.reserve_frame_response(page.page_id()).unwrap();
        let body = FrameResponseBody {
            client: client.clone(),
            page_id: page.page_id(),
            id: 17,
            lease,
        };
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        };
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        assert_eq!(
            client.resource_job_activity().0,
            1,
            "prepared response already retains activity"
        );
        let stream = body
            .decoded(creator, Some("text/html".into()), None, &budget, None)
            .unwrap();
        assert_eq!(
            client.resource_job_activity().0,
            1,
            "unpolled body is still active network custody"
        );
        let ended = std::time::Instant::now();
        drop(stream);
        client
            .drain_script_cleanup(std::time::Duration::from_secs(2))
            .unwrap();
        let (active, changed) = client.resource_job_activity();
        assert_eq!(active, 0);
        assert!(changed.is_some_and(|at| at >= ended));
    }
    #[test]
    fn frame_chunk_bounds_and_terminal_empty_are_distinct() {
        assert!(decode_chunk("", false).is_err());
        assert_eq!(decode_chunk("", true).unwrap(), (vec![], true));
        let encode = |bytes: &[u8]| {
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
        };
        assert_eq!(
            decode_chunk(&encode(&vec![1; 64 * 1024]), false)
                .unwrap()
                .0
                .len(),
            64 * 1024
        );
        assert!(decode_chunk(&encode(&vec![1; 64 * 1024 + 1]), false).is_err());
        assert!(decode_chunk("!!!!", true).is_err());
    }
    #[test]
    fn retained_frame_decoder_replays_without_releasing_before_transport_terminal() {
        use crate::host::DocumentInput;
        use std::task::Poll;
        let context = crate::runtime::Browser::new().new_context();
        let page = context.new_page();
        let client = SharedContextClient::direct(context, page.page_id());
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let lease = client.reserve_frame_response(page.page_id()).unwrap();
        let encoding = Arc::new(std::sync::Mutex::new(None));
        let timing = oryn_js::navigation_timing::NavigationTiming::new(
            std::time::Instant::now() - std::time::Duration::from_millis(1),
            oryn_js::navigation_timing::Kind::Navigate,
        );
        timing.response(
            "https://frame.test/".into(),
            200,
            &[("content-encoding".into(), "gzip".into())],
            timing.origin().0,
            std::time::Instant::now(),
            None,
            false,
        );
        let mut input = DecodedFrameInput {
            timing: Some(timing.clone()),
            received_bytes: 0,
            replay: Default::default(),
            body: FrameResponseBody {
                client: client.clone(),
                page_id: page.page_id(),
                id: 17,
                lease,
            },
            creator: oryn_js::worker_wire::Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
            pending: None,
            decoder: oryn_net::IncrementalTextDecoder::new(
                Some("text/html"),
                oryn_net::TextResourceKind::Html,
            ),
            encoding,
            done: false,
            _bytes: ScriptSourceReservation::reserve(&budget, 264 * 1024).unwrap(),
        };
        let mut prefix = vec![b' '; 1100];
        prefix.push(0xc0);
        let (tx, rx) = std::sync::mpsc::channel();
        input.pending = Some(rx);
        tx.send(Ok((prefix, false, None))).unwrap();
        assert!(matches!(input.poll_next(), Poll::Ready(Some(Ok(_)))));
        assert_eq!(client.resource_job_activity().0, 1);
        assert_eq!(timing.snapshot().values["responseEnd"], 0.);
        assert!(
            input
                .restart_encoding("windows-1251")
                .unwrap()
                .unwrap()
                .ends_with('А')
        );
        assert_eq!(
            client.resource_job_activity().0,
            1,
            "decoder restart does not acknowledge transport EOF"
        );
        let (tx, rx) = std::sync::mpsc::channel();
        input.pending = Some(rx);
        tx.send(Ok((vec![0xc1], true, Some(73)))).unwrap();
        let Poll::Ready(Some(Ok(tail))) = input.poll_next() else {
            panic!("terminal frame bytes")
        };
        assert_eq!(tail, "Б");
        assert_eq!(timing.snapshot().values["decodedBodySize"], 1102.);
        assert_eq!(timing.snapshot().values["encodedBodySize"], 73.);
        assert!(timing.snapshot().values["responseEnd"] > 0.);
        assert_eq!(client.resource_job_activity().0, 0);
        assert!(matches!(input.poll_next(), Poll::Ready(None)));
    }
}
