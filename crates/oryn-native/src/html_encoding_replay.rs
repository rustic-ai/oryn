//! Bounded native byte custody for one tentative HTML parse. Replaying never
//! requests the resource again and never treats a transport error as EOF.
use crate::host::HostError;
use oryn_net::{HtmlEncodingConfidence, IncrementalTextDecoder};
use std::sync::atomic::{AtomicUsize, Ordering};
static REPLAY_BYTES: AtomicUsize = AtomicUsize::new(0);
const DOCUMENT_LIMIT: usize = 16 * 1024 * 1024;
const AGGREGATE_LIMIT: usize = 64 * 1024 * 1024;
#[derive(Default)]
pub(crate) struct HtmlEncodingReplay {
    bytes: Vec<u8>,
    finished: bool,
}
impl Drop for HtmlEncodingReplay {
    fn drop(&mut self) {
        self.clear()
    }
}
impl HtmlEncodingReplay {
    fn clear(&mut self) {
        REPLAY_BYTES.fetch_sub(self.bytes.len(), Ordering::AcqRel);
        self.bytes = Vec::new();
    }
    pub(crate) fn commit(&mut self, decoder: &mut IncrementalTextDecoder) {
        decoder.commit_html_encoding();
        self.clear();
    }
    pub(crate) fn push(
        &mut self,
        decoder: &mut IncrementalTextDecoder,
        bytes: &[u8],
    ) -> Result<String, HostError> {
        if decoder.html_encoding_confidence() == HtmlEncodingConfidence::Tentative {
            if self
                .bytes
                .len()
                .checked_add(bytes.len())
                .is_none_or(|size| size > DOCUMENT_LIMIT)
            {
                return Err(HostError::LimitExceeded {
                    resource: "tentative HTML replay bytes".into(),
                });
            }
            let reserved = REPLAY_BYTES
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    used.checked_add(bytes.len())
                        .filter(|size| *size <= AGGREGATE_LIMIT)
                })
                .is_ok();
            if !reserved {
                return Err(HostError::LimitExceeded {
                    resource: "aggregate tentative HTML replay bytes".into(),
                });
            }
            if self.bytes.try_reserve(bytes.len()).is_err() {
                REPLAY_BYTES.fetch_sub(bytes.len(), Ordering::AcqRel);
                return Err(HostError::LimitExceeded {
                    resource: "HTML replay allocation".into(),
                });
            }
            self.bytes.extend_from_slice(bytes);
        }
        let text = decoder.push(bytes);
        if decoder.html_encoding_confidence() == HtmlEncodingConfidence::Certain {
            self.clear()
        }
        Ok(text)
    }
    pub(crate) fn finish(&mut self, decoder: &mut IncrementalTextDecoder) -> String {
        self.finished = true;
        let text = decoder.finish();
        if decoder.html_encoding_confidence() == HtmlEncodingConfidence::Certain {
            self.clear()
        }
        text
    }
    pub(crate) fn restart(
        &mut self,
        decoder: &mut IncrementalTextDecoder,
        label: &str,
    ) -> Option<String> {
        let replay = decoder.restart_html_encoding(label, &self.bytes, self.finished);
        if decoder.html_encoding_confidence() == HtmlEncodingConfidence::Certain {
            self.clear()
        }
        replay
    }
}

pub(crate) struct BufferedDecodedInput {
    bytes: Option<Vec<u8>>,
    decoder: IncrementalTextDecoder,
    replay: HtmlEncodingReplay,
    encoding: std::sync::Arc<std::sync::Mutex<Option<&'static str>>>,
    capture: Option<Box<dyn FnMut(&str)>>,
}
impl BufferedDecodedInput {
    pub(crate) fn stream(
        bytes: Vec<u8>,
        content_type: Option<String>,
        fallback: Option<&str>,
        capture: Option<Box<dyn FnMut(&str)>>,
    ) -> crate::host::DocumentStream {
        let encoding = std::sync::Arc::new(std::sync::Mutex::new(None));
        let kind = crate::network::navigation_text_kind(content_type.as_deref());
        let decoder =
            IncrementalTextDecoder::with_fallback(content_type.as_deref(), kind, fallback)
                .with_html_detection();
        crate::host::DocumentStream {
            content_language: None,
            content_type,
            chunks: Box::new(Self {
                bytes: Some(bytes),
                decoder,
                replay: Default::default(),
                encoding: encoding.clone(),
                capture,
            }),
            encoding,
        }
    }
    fn publish(&mut self, text: &str) {
        *self.encoding.lock().expect("buffered document encoding") = self.decoder.encoding_name();
        if let Some(capture) = &mut self.capture {
            capture(text)
        }
    }
}
impl crate::host::DocumentInput for BufferedDecodedInput {
    fn poll_next(&mut self) -> std::task::Poll<Option<Result<String, HostError>>> {
        let Some(bytes) = self.bytes.take() else {
            return std::task::Poll::Ready(None);
        };
        let result = self.replay.push(&mut self.decoder, &bytes).map(|mut text| {
            text.push_str(&self.replay.finish(&mut self.decoder));
            self.publish(&text);
            text
        });
        std::task::Poll::Ready(Some(result))
    }
    fn encoding_confidence(&self) -> HtmlEncodingConfidence {
        self.decoder.html_encoding_confidence()
    }
    fn commit_encoding(&mut self) {
        self.replay.commit(&mut self.decoder)
    }
    fn encoding_restart_needed(&self, label: &str) -> bool {
        self.decoder.html_encoding_restart_needed(label)
    }
    fn restart_encoding(&mut self, label: &str) -> Result<Option<String>, HostError> {
        let replay = self.replay.restart(&mut self.decoder, label);
        if let Some(text) = &replay {
            self.publish(text)
        } else {
            *self.encoding.lock().expect("buffered document encoding") =
                self.decoder.encoding_name();
        }
        Ok(replay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tentative_replay_preserves_every_split_and_commits_once() {
        let mut raw = format!(
            "<head>{}<meta charset=windows-1251></head><body>",
            " ".repeat(1100)
        )
        .into_bytes();
        raw.extend_from_slice(&[0xc0, 0xc1]);
        for split in 0..=raw.len() {
            let mut decoder =
                IncrementalTextDecoder::new(Some("text/html"), oryn_net::TextResourceKind::Html);
            let mut replay = HtmlEncodingReplay::default();
            let _ = replay.push(&mut decoder, &raw[..split]).unwrap();
            let _ = replay.push(&mut decoder, &raw[split..]).unwrap();
            let _ = replay.finish(&mut decoder);
            let text = replay.restart(&mut decoder, "windows-1251").unwrap();
            assert!(text.ends_with("АБ"), "split {split}");
            assert!(replay.bytes.is_empty());
            assert!(!decoder.html_encoding_restart_needed("utf-8"));
            assert!(replay.restart(&mut decoder, "utf-8").is_none());
        }
    }
    #[test]
    fn replay_budget_failure_keeps_accepted_prefix_and_explicit_commit_releases_it() {
        let mut decoder =
            IncrementalTextDecoder::new(Some("text/html"), oryn_net::TextResourceKind::Html);
        let mut replay = HtmlEncodingReplay::default();
        replay.push(&mut decoder, b"<head>").unwrap();
        let before = replay.bytes.clone();
        assert!(
            replay
                .push(&mut decoder, &vec![b' '; DOCUMENT_LIMIT])
                .is_err()
        );
        assert_eq!(replay.bytes, before);
        replay.commit(&mut decoder);
        assert!(replay.bytes.is_empty());
        assert!(replay.restart(&mut decoder, "windows-1251").is_none());
    }
    #[test]
    fn buffered_input_keeps_raw_bytes_until_parser_commits() {
        let mut raw = vec![b' '; 1100];
        raw.push(0xc0);
        let mut stream = BufferedDecodedInput::stream(raw, Some("text/html".into()), None, None);
        assert_eq!(
            stream.chunks.encoding_confidence(),
            HtmlEncodingConfidence::Tentative
        );
        assert!(matches!(
            stream.chunks.poll_next(),
            std::task::Poll::Ready(Some(Ok(_)))
        ));
        assert!(matches!(
            stream.chunks.poll_next(),
            std::task::Poll::Ready(None)
        ));
        assert!(
            stream
                .chunks
                .restart_encoding("windows-1251")
                .unwrap()
                .unwrap()
                .ends_with('А')
        );
        assert_eq!(*stream.encoding.lock().unwrap(), Some("windows-1251"));
    }
    #[test]
    fn restart_preflight_leaves_original_decoder_and_prefix_unchanged() {
        let mut decoder =
            IncrementalTextDecoder::new(Some("text/html"), oryn_net::TextResourceKind::Html);
        let mut replay = HtmlEncodingReplay::default();
        let mut raw = vec![b' '; 1100];
        raw.push(0xc0);
        replay.push(&mut decoder, &raw).unwrap();
        assert_eq!(decoder.encoding_name(), Some("windows-1252"));
        assert!(decoder.html_encoding_restart_needed("windows-1251"));
        assert_eq!(replay.bytes, raw);
        assert_eq!(decoder.encoding_name(), Some("windows-1252"));
        assert_eq!(
            decoder.html_encoding_confidence(),
            HtmlEncodingConfidence::Tentative
        );
        replay.commit(&mut decoder);
        assert!(!decoder.html_encoding_restart_needed("windows-1251"));
    }
}
