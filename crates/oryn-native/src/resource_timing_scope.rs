//! Timing custody for external resource loads performed outside PlatformHost.
use super::{ContextServiceRequest as Request, ContextServiceResponse as Response, RuntimeError};
#[cfg(any(feature = "v8-host", test))]
use oryn_js::worker_wire::Creator;
use std::{
    collections::{BTreeMap, VecDeque},
    time::Instant,
};
pub(crate) struct Record {
    pub render_blocking: bool,
    pub details: Option<oryn_js::platform::ResourceTimingMetadata>,
    #[cfg(any(feature = "v8-host", test))]
    pub creator: Creator,
    #[cfg(feature = "v8-host")]
    pub name: String,
    pub initiator: String,
    #[cfg(any(feature = "v8-host", test))]
    pub started: Instant,
    pub finished: Instant,
}
pub(super) enum Observation {
    Start(Box<Record>),
    Read(u64),
    Abort(u64),
}
#[derive(Default)]
pub(super) struct Queue {
    ready: VecDeque<Record>,
    streams: BTreeMap<u64, Record>,
}
pub(super) fn observe(request: &Request) -> Option<Observation> {
    match request {
        Request::PreloadResource { request, .. } => observe(request),
        Request::RunScriptLoad {
            request,
            render_blocking,
            ..
        } => {
            let mut observation = observe(request)?;
            if let Observation::Start(record) = &mut observation {
                record.render_blocking |= *render_blocking;
            }
            Some(observation)
        }
        Request::LoadScript {
            creator: _creator,
            url: _url,
            ..
        }
        | Request::LoadTypedModule {
            creator: _creator,
            url: _url,
            ..
        } => Some(Observation::Start(Box::new(Record {
            render_blocking: false,
            details: None,
            #[cfg(any(feature = "v8-host", test))]
            creator: _creator.clone(),
            #[cfg(feature = "v8-host")]
            name: _url.clone(),
            initiator: "script".into(),
            #[cfg(any(feature = "v8-host", test))]
            started: Instant::now(),
            finished: Instant::now(),
        }))),
        Request::LoadPageResource {
            initiator,
            url: _url,
            destination,
            ..
        } => Some(Observation::Start(Box::new(Record {
            render_blocking: matches!(destination.as_str(), "style" | "stylesheet")
                && initiator
                    .stylesheet
                    .as_ref()
                    .is_some_and(|options| options.render_blocking),
            details: None,
            #[cfg(any(feature = "v8-host", test))]
            creator: initiator.creator.clone(),
            #[cfg(feature = "v8-host")]
            name: _url.clone(),
            initiator: match destination.as_str() {
                "style" | "stylesheet" => "link",
                "font" => "css",
                "frame" | "iframe" => "iframe",
                _ => destination,
            }
            .into(),
            #[cfg(any(feature = "v8-host", test))]
            started: Instant::now(),
            finished: Instant::now(),
        }))),
        Request::TemplateStreamStart { request, .. } => request.client.as_ref().map(|_creator| {
            Observation::Start(Box::new(Record {
                render_blocking: false,
                details: None,
                #[cfg(any(feature = "v8-host", test))]
                creator: _creator.clone(),
                #[cfg(feature = "v8-host")]
                name: request.url.clone(),
                initiator: "template".into(),
                #[cfg(any(feature = "v8-host", test))]
                started: Instant::now(),
                finished: Instant::now(),
            }))
        }),
        Request::ReleaseNetworkResult {
            stream_id: Some(stream_id),
            ..
        } => Some(Observation::Abort(*stream_id)),
        Request::HttpStreamRead { stream_id, .. } => Some(Observation::Read(*stream_id)),
        Request::HttpStreamAbort { stream_id, .. } => Some(Observation::Abort(*stream_id)),
        _ => None,
    }
}
impl Queue {
    fn push(&mut self, mut record: Record) {
        record.finished = Instant::now();
        if self.ready.len() < 250 {
            self.ready.push_back(record)
        }
    }
    pub(super) fn finish(
        &mut self,
        observation: Observation,
        result: &Result<Response, RuntimeError>,
    ) {
        match observation {
            Observation::Start(record) => {
                let mut record = *record;
                record.details = match result {
                    Ok(Response::PreloadedResource { response, .. }) => match response.as_ref() {
                        Response::ScriptSource { timing, .. } => timing.clone(),
                        Response::PageResource { timing, .. } => timing.as_deref().cloned(),
                        _ => None,
                    },
                    Ok(
                        Response::ScriptSource { timing, .. }
                        | Response::PageBinaryResource { timing, .. }
                        | Response::HttpStreamStarted { timing, .. },
                    ) => timing.clone(),
                    Ok(Response::PageResource { timing, .. }) => timing.as_deref().cloned(),
                    _ => None,
                };
                if let Ok(
                    Response::PageResource {
                        stream_id: Some(stream),
                        ..
                    }
                    | Response::HttpStreamStarted {
                        stream_id: stream, ..
                    },
                ) = result
                {
                    if self.streams.len() < 256 {
                        self.streams.insert(*stream, record);
                    }
                } else {
                    self.push(record)
                }
            }
            Observation::Read(stream) => {
                if matches!(result,Ok(Response::HttpStreamChunk{stream_id,..})if *stream_id!=stream)
                {
                    return;
                }
                if let Ok(Response::HttpStreamChunk {
                    body_base64,
                    done,
                    encoded_body_size,
                    ..
                }) = result
                    && let Some(record) = self.streams.get_mut(&stream)
                    && let Some(details) = &mut record.details
                {
                    let bytes = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        body_base64,
                    )
                    .map_or(0, |bytes| bytes.len() as u64);
                    details.decoded_body_size =
                        Some(details.decoded_body_size.unwrap_or(0).saturating_add(bytes));
                    if *done {
                        details.encoded_body_size = encoded_body_size.or(details.encoded_body_size);
                    }
                }
                if !matches!(result, Ok(Response::HttpStreamChunk { done: false, .. }))
                    && let Some(record) = self.streams.remove(&stream)
                {
                    self.push(record)
                }
            }
            Observation::Abort(stream) => {
                if matches!(
                    result,
                    Ok(Response::HttpStreamAborted | Response::NetworkResultReleased)
                ) && let Some(record) = self.streams.remove(&stream)
                {
                    self.push(record)
                }
            }
        }
    }
    #[cfg(any(feature = "v8-host", test))]
    pub(super) fn drain(&mut self) -> Vec<Record> {
        self.ready.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::message_wire::WindowTarget;
    fn record() -> Record {
        Record {
            render_blocking: false,
            details: None,
            creator: Creator::Window {
                target: WindowTarget::Root,
                document_generation: 1,
                stream_epoch: 0,
            },
            #[cfg(feature = "v8-host")]
            name: "https://example.test/frame".into(),
            initiator: "iframe".into(),
            started: Instant::now(),
            finished: Instant::now(),
        }
    }
    #[test]
    fn frame_timing_waits_for_real_stream_terminal() {
        let mut queue = Queue::default();
        queue.streams.insert(7, record());
        queue.finish(
            Observation::Read(7),
            &Ok(Response::HttpStreamChunk {
                encoded_body_size: None,
                stream_id: 7,
                body_base64: "YQ==".into(),
                done: false,
            }),
        );
        assert!(queue.drain().is_empty());
        queue.finish(
            Observation::Read(7),
            &Ok(Response::HttpStreamChunk {
                encoded_body_size: None,
                stream_id: 7,
                body_base64: String::new(),
                done: true,
            }),
        );
        let entries = queue.drain();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].finished >= entries[0].started);
        queue.finish(
            Observation::Read(7),
            &Ok(Response::HttpStreamChunk {
                encoded_body_size: None,
                stream_id: 7,
                body_base64: String::new(),
                done: true,
            }),
        );
        assert!(queue.drain().is_empty());
    }
    #[test]
    fn foreign_stream_reply_cannot_complete_the_observed_request() {
        let mut queue = Queue::default();
        queue.streams.insert(7, record());
        queue.finish(
            Observation::Read(7),
            &Ok(Response::HttpStreamChunk {
                encoded_body_size: Some(42),
                stream_id: 8,
                body_base64: "YQ==".into(),
                done: true,
            }),
        );
        assert!(queue.drain().is_empty());
        assert!(queue.streams.contains_key(&7));
        queue.finish(
            Observation::Read(7),
            &Ok(Response::HttpStreamChunk {
                encoded_body_size: Some(1),
                stream_id: 7,
                body_base64: "YQ==".into(),
                done: true,
            }),
        );
        assert_eq!(queue.drain().len(), 1);
        assert!(queue.streams.is_empty());
    }
    #[test]
    fn template_response_waits_for_terminal_and_cleanup_does_not_duplicate_timing() {
        let mut queue = Queue::default();
        let mut admitted = record();
        admitted.initiator = "template".into();
        queue.finish(
            Observation::Start(Box::new(admitted)),
            &Ok(Response::HttpStreamStarted {
                timing: None,
                internal_response: None,
                response_type: None,
                upload: None,
                cors_filtered: false,
                url: "https://example.test/fragment".into(),
                redirected: false,
                stream_id: 31,
                request_id: 9,
                status: 200,
                headers: Vec::new(),
                total_bytes: None,
            }),
        );
        assert!(queue.drain().is_empty());
        queue.finish(
            Observation::Read(31),
            &Ok(Response::HttpStreamChunk {
                encoded_body_size: None,
                stream_id: 31,
                body_base64: "YQ==".into(),
                done: false,
            }),
        );
        assert!(queue.drain().is_empty());
        queue.finish(Observation::Abort(31), &Ok(Response::NetworkResultReleased));
        let rows = queue.drain();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].initiator, "template");
        queue.finish(
            Observation::Read(31),
            &Ok(Response::HttpStreamChunk {
                encoded_body_size: Some(1),
                stream_id: 31,
                body_base64: String::new(),
                done: true,
            }),
        );
        assert!(queue.drain().is_empty());
    }
    #[test]
    fn unrelated_reads_do_not_create_duplicate_platform_entries() {
        let mut queue = Queue::default();
        queue.finish(
            Observation::Read(17),
            &Ok(Response::HttpStreamChunk {
                encoded_body_size: None,
                stream_id: 17,
                body_base64: String::new(),
                done: true,
            }),
        );
        assert!(queue.drain().is_empty());
    }
    #[test]
    fn script_admission_status_survives_failure_without_response_metadata() {
        let request = Request::RunScriptLoad {
            render_blocking: true,
            page_id: oryn_common::v3::PageId(1),
            token: 7,
            request: Box::new(Request::LoadScript {
                fallback_encoding: None,
                parser_inserted: false,
                script_policy: None,
                page_id: oryn_common::v3::PageId(1),
                creator: record().creator,
                url: "https://example.test/blocking.js".into(),
                module: false,
                nonce: None,
                integrity: None,
                cross_origin: None,
                initiator_url: None,
                referrer_policy: None,
                referrer_source: None,
            }),
        };
        let mut queue = Queue::default();
        queue.finish(
            observe(&request).unwrap(),
            &Err(RuntimeError::ContextService("resource refused".into())),
        );
        let records = queue.drain();
        assert_eq!(records.len(), 1);
        assert!(records[0].render_blocking);
        assert!(records[0].details.is_none());
        let Request::RunScriptLoad { request, .. } = request else {
            unreachable!()
        };
        let Observation::Start(unadmitted) = observe(&request).unwrap() else {
            unreachable!()
        };
        assert!(!unadmitted.render_blocking);
    }
}
