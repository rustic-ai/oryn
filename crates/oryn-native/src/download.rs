//! Parent-owned quarantine for page-initiated downloads.
//!
//! Download bytes are never written to a host path implicitly. A caller must
//! explicitly release an item from this store and decide where it belongs.

use std::{collections::BTreeMap, fmt, sync::Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DownloadId([u8; 32]);

impl fmt::Debug for DownloadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "DownloadId({})", hex_prefix(&self.0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadDescriptor {
    pub id: DownloadId,
    pub source_url: String,
    pub source_origin: String,
    pub suggested_filename: String,
    pub media_type: String,
    pub bytes: usize,
    pub sha256: String,
    pub quarantined: bool,
}

struct DownloadEntry {
    descriptor: DownloadDescriptor,
    body: Vec<u8>,
}

#[derive(Default)]
pub struct DownloadStore {
    entries: Mutex<BTreeMap<DownloadId, DownloadEntry>>,
}

impl DownloadStore {
    pub fn quarantine(
        &self,
        source_url: String,
        source_origin: String,
        suggested_filename: String,
        media_type: String,
        body: Vec<u8>,
        max_bytes: usize,
    ) -> Result<DownloadDescriptor, DownloadError> {
        if body.len() > max_bytes {
            return Err(DownloadError::TooLarge {
                limit: max_bytes,
                actual: body.len(),
            });
        }
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random).map_err(|_| DownloadError::Entropy)?;
        let id = DownloadId(random);
        let descriptor = DownloadDescriptor {
            id,
            source_url,
            source_origin,
            suggested_filename: sanitize_filename(&suggested_filename),
            media_type,
            bytes: body.len(),
            sha256: format!("{:x}", Sha256::digest(&body)),
            quarantined: true,
        };
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| DownloadError::Unavailable)?;
        if entries.contains_key(&id) {
            return Err(DownloadError::Collision);
        }
        let retained = entries
            .values()
            .try_fold(body.len(), |bytes, entry| {
                bytes.checked_add(entry.body.len())
            })
            .ok_or(DownloadError::Capacity)?;
        if entries.len() >= 1024 || retained > 256 * 1024 * 1024 {
            return Err(DownloadError::Capacity);
        }
        entries.insert(
            id,
            DownloadEntry {
                descriptor: descriptor.clone(),
                body,
            },
        );
        Ok(descriptor)
    }

    pub fn list(&self) -> Vec<DownloadDescriptor> {
        self.entries
            .lock()
            .expect("download store lock")
            .values()
            .map(|entry| entry.descriptor.clone())
            .collect()
    }

    /// Explicitly remove an item from quarantine and return its bytes to the
    /// trusted host. This API never chooses or writes a filesystem path.
    pub fn release(&self, id: DownloadId) -> Result<Vec<u8>, DownloadError> {
        self.entries
            .lock()
            .expect("download store lock")
            .remove(&id)
            .map(|entry| entry.body)
            .ok_or(DownloadError::Unknown)
    }

    pub fn discard(&self, id: DownloadId) -> bool {
        self.entries
            .lock()
            .expect("download store lock")
            .remove(&id)
            .is_some()
    }
}

/// RFC 6266 response disposition plus an already-authorized download attribute.
/// Invalid/duplicate parameters never select an ambiguous filename. A valid
/// attachment (including an unknown disposition type) still downloads using a
/// safe fallback name. Multiple field lines invalidate filename parameters.
pub(crate) fn response_is_download(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-disposition"))
        .any(|(_, field)| {
            disposition_type(field).is_some_and(|kind| !kind.eq_ignore_ascii_case("inline"))
        })
}
pub(crate) fn response_download_filename(
    headers: &[(String, String)],
    url: &url::Url,
    attribute: Option<&str>,
) -> Option<String> {
    let fields = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-disposition"))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    let attachment = response_is_download(headers);
    if !attachment && attribute.is_none() {
        return None;
    }
    let header_name = if fields.len() == 1 {
        disposition_parameters(fields[0]).and_then(|parameters| {
            parameters
                .get("filename*")
                .filter(|(_, quoted)| !*quoted)
                .and_then(|(value, _)| decode_extended_filename(value))
                .or_else(|| parameters.get("filename").map(|(value, _)| value.clone()))
        })
    } else {
        None
    };
    let candidate = header_name
        .filter(|name| !name.is_empty())
        .or_else(|| attribute.filter(|name| !name.is_empty()).map(str::to_owned))
        .unwrap_or_else(|| {
            let segment = url
                .path_segments()
                .and_then(|segments| segments.last())
                .unwrap_or("");
            percent_decode_filename(segment)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_else(|| segment.into())
        });
    Some(sanitize_filename(&candidate))
}
fn disposition_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}
fn disposition_type(value: &str) -> Option<&str> {
    let value = value.trim_start_matches([' ', '\t']);
    let end = value
        .bytes()
        .position(|byte| !disposition_token(byte))
        .unwrap_or(value.len());
    if end == 0 {
        return None;
    }
    let rest = value[end..].trim_start_matches([' ', '\t']);
    if !rest.is_empty() && !rest.starts_with(';') {
        return None;
    }
    Some(&value[..end])
}
fn disposition_parameters(value: &str) -> Option<BTreeMap<String, (String, bool)>> {
    if value.len() > 65536 {
        return None;
    }
    let value = value.trim_start_matches([' ', '\t']);
    let kind = disposition_type(value)?;
    let bytes = value.as_bytes();
    let mut position = kind.len();
    let mut parameters = BTreeMap::new();
    while position < bytes.len() {
        while bytes
            .get(position)
            .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
        {
            position += 1;
        }
        if position == bytes.len() {
            break;
        }
        if bytes[position] != b';' {
            return None;
        }
        position += 1;
        while bytes
            .get(position)
            .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
        {
            position += 1;
        }
        let start = position;
        while bytes
            .get(position)
            .is_some_and(|byte| disposition_token(*byte))
        {
            position += 1;
        }
        if position == start {
            return None;
        }
        let name = value[start..position].to_ascii_lowercase();
        while bytes
            .get(position)
            .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
        {
            position += 1;
        }
        if bytes.get(position) != Some(&b'=') {
            return None;
        }
        position += 1;
        while bytes
            .get(position)
            .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
        {
            position += 1;
        }
        let quoted = bytes.get(position) == Some(&b'"');
        let parameter = if quoted {
            position += 1;
            let mut output = Vec::new();
            let mut closed = false;
            while let Some(&byte) = bytes.get(position) {
                position += 1;
                if byte == b'"' {
                    closed = true;
                    break;
                }
                let byte = if byte == b'\\' {
                    let byte = *bytes.get(position)?;
                    position += 1;
                    byte
                } else {
                    byte
                };
                if (byte < 0x20 && byte != b'\t') || byte == 0x7f {
                    return None;
                }
                output.push(byte);
            }
            if !closed {
                return None;
            }
            String::from_utf8(output).ok()?
        } else {
            let start = position;
            while bytes
                .get(position)
                .is_some_and(|byte| disposition_token(*byte))
            {
                position += 1;
            }
            if start == position {
                return None;
            }
            value[start..position].to_string()
        };
        if parameters.insert(name, (parameter, quoted)).is_some() {
            return None;
        }
        // The next iteration accepts only OWS followed by a semicolon/end.
    }
    Some(parameters)
}
fn percent_decode_filename(value: &str) -> Option<Vec<u8>> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = (*bytes.get(index + 1)? as char).to_digit(16)?;
            let low = (*bytes.get(index + 2)? as char).to_digit(16)?;
            output.push((high * 16 + low) as u8);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    Some(output)
}
fn decode_extended_filename(value: &str) -> Option<String> {
    let mut pieces = value.splitn(3, '\'');
    let charset = pieces.next()?;
    let language = pieces.next()?;
    let encoded = pieces.next()?;
    if !language
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return None;
    }
    if !encoded
        .bytes()
        .all(|byte| byte == b'%' || byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte))
    {
        return None;
    }
    let bytes = percent_decode_filename(encoded)?;
    if charset.eq_ignore_ascii_case("UTF-8") {
        String::from_utf8(bytes).ok()
    } else if charset.eq_ignore_ascii_case("ISO-8859-1") {
        Some(bytes.into_iter().map(char::from).collect())
    } else {
        None
    }
}

fn sanitize_filename(filename: &str) -> String {
    let name = filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("download")
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    if name.is_empty() || name == "." || name == ".." {
        "download".into()
    } else {
        name
    }
}

fn hex_prefix(bytes: &[u8; 32]) -> String {
    bytes[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DownloadError {
    #[error("download capability entropy unavailable")]
    Entropy,
    #[error("download quarantine lock unavailable")]
    Unavailable,
    #[error("download quarantine capacity exhausted")]
    Capacity,
    #[error("download capability collision")]
    Collision,
    #[error("download exceeds quarantine limit: {actual} > {limit} bytes")]
    TooLarge { limit: usize, actual: usize },
    #[error("download is unknown or was already released")]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_name(value: &str, attribute: Option<&str>) -> Option<String> {
        response_download_filename(
            &[("Content-Disposition".into(), value.into())],
            &url::Url::parse(
                "https://files.test/path/fallback%20name.bin?ignored=filename#fragment",
            )
            .unwrap(),
            attribute,
        )
    }
    #[test]
    fn response_disposition_parses_quoted_pairs_without_splitting_semicolons() {
        assert_eq!(
            response_name(r#"ATTACHMENT; filename="report; version=2.txt""#, None).as_deref(),
            Some("report; version=2.txt")
        );
        assert_eq!(
            response_name(r#"attachment; filename="say\"hello.txt""#, None).as_deref(),
            Some("say\"hello.txt")
        );
        assert_eq!(
            response_name(r#"attachment; filename="folder\\file.txt""#, None).as_deref(),
            Some("file.txt")
        );
        assert_eq!(response_name("inline; filename=x.txt", None), None);
        assert_eq!(
            response_name("inline; filename=x.txt", Some("attribute.txt")).as_deref(),
            Some("x.txt")
        );
        assert_eq!(
            response_name("unknown-extension; filename=x.txt", None).as_deref(),
            Some("x.txt")
        );
    }
    #[test]
    fn response_disposition_prefers_valid_extended_filename_and_falls_back_on_invalid_encoding() {
        assert_eq!(
            response_name(
                "attachment; filename=plain.txt; filename*=UTF-8'en'%E2%82%AC%20rates.txt",
                None
            )
            .as_deref(),
            Some("€ rates.txt")
        );
        assert_eq!(
            response_name(
                "attachment; filename*=iso-8859-1''caf%E9.txt; filename=plain.txt",
                None
            )
            .as_deref(),
            Some("café.txt")
        );
        assert_eq!(
            response_name("attachment; filename*=UTF-8''a+b.txt", None).as_deref(),
            Some("a+b.txt")
        );
        for invalid in [
            "UTF-8''bad%ZZ.txt",
            "UTF-8''bad%E9.txt",
            "unknown''name.txt",
            "UTF-8'name",
            "UTF-8''bad%",
            r#""UTF-8''quoted.txt""#,
        ] {
            assert_eq!(
                response_name(
                    &format!("attachment; filename=plain.txt; filename*={invalid}"),
                    None
                )
                .as_deref(),
                Some("plain.txt"),
                "{invalid}"
            );
        }
        assert_eq!(
            response_name("attachment; filename=percent%20literal.txt", None).as_deref(),
            Some("percent%20literal.txt")
        );
    }
    #[test]
    fn response_disposition_rejects_ambiguous_parameters_but_preserves_attachment_classification() {
        for malformed in [
            "attachment; filename=a; FILENAME=b",
            "attachment; filename*=UTF-8''a; filename*=UTF-8''b",
            "attachment; filename=valid; x=a; X=b",
            r#"attachment; filename="unclosed"#,
            "attachment; filename=valid trailing",
            "attachment; filename=ok; broken",
            "attachment; filename=\"x\r\ny\"",
        ] {
            assert_eq!(
                response_name(malformed, None).as_deref(),
                Some("fallback name.bin"),
                "{malformed}"
            );
        }
        let url = url::Url::parse("https://files.test/fallback").unwrap();
        assert_eq!(
            response_download_filename(
                &[
                    ("content-disposition".into(), "inline; filename=one".into()),
                    (
                        "Content-Disposition".into(),
                        "attachment; filename=two".into()
                    )
                ],
                &url,
                None
            )
            .as_deref(),
            Some("fallback")
        );
        assert_eq!(
            response_name("attachment; filename=a; filename=b", Some("chosen.txt")).as_deref(),
            Some("chosen.txt")
        );
    }
    #[test]
    fn response_download_fallback_excludes_url_query_and_path_components() {
        for (url, expected) in [
            ("https://files.test/", "download"),
            ("https://files.test/%2E%2E", "download"),
            ("https://files.test/a%2Fsafe.txt?name=wrong", "safe.txt"),
            ("https://files.test/%00safe.txt", "safe.txt"),
        ] {
            assert_eq!(
                response_download_filename(&[], &url::Url::parse(url).unwrap(), Some(""))
                    .as_deref(),
                Some(expected)
            );
        }
        assert_eq!(
            response_download_filename(
                &[],
                &url::Url::parse("https://files.test/a").unwrap(),
                None
            ),
            None
        );
    }

    #[test]
    fn quarantine_capacity_rejection_preserves_existing_custody() {
        let store = DownloadStore::default();
        let mut first = None;
        for _ in 0..1024 {
            let descriptor = store
                .quarantine(
                    "https://files.test/x".into(),
                    "https://source.test".into(),
                    "x".into(),
                    "text/plain".into(),
                    Vec::new(),
                    1,
                )
                .unwrap();
            if first.is_none() {
                first = Some(descriptor.id);
            }
        }
        assert_eq!(
            store.quarantine(
                "https://files.test/y".into(),
                "https://source.test".into(),
                "y".into(),
                "text/plain".into(),
                Vec::new(),
                1
            ),
            Err(DownloadError::Capacity)
        );
        assert!(store.release(first.unwrap()).is_ok());
        assert_eq!(store.list().len(), 1023);
    }

    #[test]
    fn quarantine_never_interprets_page_filename_as_a_path() {
        let store = DownloadStore::default();
        let descriptor = store
            .quarantine(
                "https://files.example/item".into(),
                "https://files.example".into(),
                "../../secret.txt".into(),
                "text/plain".into(),
                b"payload".to_vec(),
                1024,
            )
            .unwrap();
        assert_eq!(descriptor.suggested_filename, "secret.txt");
        assert!(descriptor.quarantined);
        assert_eq!(store.release(descriptor.id).unwrap(), b"payload");
        assert_eq!(store.release(descriptor.id), Err(DownloadError::Unknown));
    }

    #[test]
    fn oversized_download_is_not_retained() {
        let store = DownloadStore::default();
        assert!(matches!(
            store.quarantine(
                "https://files.example/item".into(),
                "https://files.example".into(),
                "item.bin".into(),
                "application/octet-stream".into(),
                vec![0; 5],
                4,
            ),
            Err(DownloadError::TooLarge { .. })
        ));
        assert!(store.list().is_empty());
    }
}

/// Binary document sniffing selects the native download quarantine; it never
/// changes the supplied Content-Type visible through Fetch.
pub(crate) fn computed_type_is_download(mime: &str) -> bool {
    matches!(
        mime,
        "application/octet-stream"
            | "application/zip"
            | "application/x-gzip"
            | "application/x-rar-compressed"
            | "application/postscript"
    )
}
