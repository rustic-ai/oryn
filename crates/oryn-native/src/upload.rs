//! Parent-owned upload store. Host paths and opaque handles never cross the
//! page-worker protocol; only explicitly selected file payloads do.

use std::{collections::BTreeMap, fmt, path::Path, sync::Mutex};

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UploadHandle([u8; 32]);

impl fmt::Debug for UploadHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UploadHandle([OPAQUE])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadDescriptor {
    pub filename: String,
    pub media_type: String,
    pub byte_length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadFile {
    pub filename: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// Original file metadata retained by authenticated WebDriver selection.
/// Contains bytes and a display name; it never contains a host path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebDriverUpload {
    pub file: UploadFile,
    pub last_modified: i64,
}

pub(crate) fn file_media_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "txt" | "text" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "csv" => "text/csv",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "zip" => "application/zip",
        "wasm" => "application/wasm",
        _ => "",
    }
}

struct UploadEntry {
    file: UploadFile,
}

impl Drop for UploadEntry {
    fn drop(&mut self) {
        self.file.bytes.zeroize();
    }
}

#[derive(Default)]
pub struct UploadStore {
    entries: Mutex<BTreeMap<UploadHandle, UploadEntry>>,
    labels: Mutex<BTreeMap<String, UploadHandle>>,
}

impl UploadStore {
    pub fn insert(&self, file: UploadFile) -> UploadHandle {
        let label = file.filename.clone();
        self.insert_named(label, file)
    }

    pub fn insert_named(&self, label: String, file: UploadFile) -> UploadHandle {
        let mut handle = [0_u8; 32];
        getrandom::fill(&mut handle).expect("operating-system randomness is required");
        let handle = UploadHandle(handle);
        self.entries
            .lock()
            .expect("upload store lock")
            .insert(handle, UploadEntry { file });
        self.labels
            .lock()
            .expect("upload label lock")
            .insert(label, handle);
        handle
    }

    pub fn import_path(
        &self,
        path: &Path,
        media_type: Option<&str>,
    ) -> std::io::Result<UploadHandle> {
        let bytes = std::fs::read(path)?;
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("upload.bin")
            .to_owned();
        Ok(self.insert(UploadFile {
            filename,
            media_type: media_type.unwrap_or("application/octet-stream").to_owned(),
            bytes,
        }))
    }

    pub fn materialize(&self, handle: UploadHandle) -> Result<UploadFile, UploadError> {
        Ok(self.materialize_batch(&[handle])?.remove(0))
    }

    /// Validate the complete selected set under one grant lock before cloning
    /// payloads. Revocation cannot split one admission into a partial selection.
    pub fn materialize_batch(
        &self,
        handles: &[UploadHandle],
    ) -> Result<Vec<UploadFile>, UploadError> {
        if handles.is_empty() || handles.len() > 128 {
            return Err(UploadError::SelectionLimit);
        }
        let entries = self.entries.lock().expect("upload store lock");
        let mut bytes = 0usize;
        for handle in handles {
            let file = &entries.get(handle).ok_or(UploadError::UnknownHandle)?.file;
            bytes = bytes
                .checked_add(file.bytes.len())
                .ok_or(UploadError::SelectionLimit)?;
            if bytes > 64 * 1024 * 1024
                || file.filename.len() > 65_536
                || file.media_type.len() > 65_536
            {
                return Err(UploadError::SelectionLimit);
            }
        }
        Ok(handles
            .iter()
            .map(|handle| entries[handle].file.clone())
            .collect())
    }

    pub fn descriptor(&self, handle: UploadHandle) -> Result<UploadDescriptor, UploadError> {
        let entries = self.entries.lock().expect("upload store lock");
        let file = &entries.get(&handle).ok_or(UploadError::UnknownHandle)?.file;
        Ok(UploadDescriptor {
            filename: file.filename.clone(),
            media_type: file.media_type.clone(),
            byte_length: file.bytes.len(),
        })
    }

    pub fn revoke(&self, handle: UploadHandle) -> bool {
        let removed = self
            .entries
            .lock()
            .expect("upload store lock")
            .remove(&handle)
            .is_some();
        if removed {
            self.labels
                .lock()
                .expect("upload label lock")
                .retain(|_, candidate| *candidate != handle);
        }
        removed
    }

    pub fn handle_for_label(&self, label: &str) -> Result<UploadHandle, UploadError> {
        self.labels
            .lock()
            .expect("upload label lock")
            .get(label)
            .copied()
            .ok_or(UploadError::UnknownLabel)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UploadError {
    #[error("file selection exceeds count, metadata or byte limits")]
    SelectionLimit,
    #[error("upload handle is unknown or revoked")]
    UnknownHandle,
    #[error("upload label is unknown")]
    UnknownLabel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_selection_is_bounded_and_revocation_does_not_change_owned_bytes() {
        let store = UploadStore::default();
        let first = store.insert(UploadFile {
            filename: "one".into(),
            media_type: "text/plain".into(),
            bytes: b"first".to_vec(),
        });
        let second = store.insert(UploadFile {
            filename: "two".into(),
            media_type: "text/plain".into(),
            bytes: b"second".to_vec(),
        });
        let selected = store.materialize_batch(&[first, second]).unwrap();
        assert!(store.revoke(second));
        assert_eq!(selected[1].bytes, b"second");
        assert_eq!(
            store.materialize_batch(&[first, second]),
            Err(UploadError::UnknownHandle)
        );
        assert_eq!(
            store.materialize_batch(&vec![first; 129]),
            Err(UploadError::SelectionLimit)
        );
        let oversized_name = store.insert(UploadFile {
            filename: "n".repeat(65_537),
            media_type: String::new(),
            bytes: Vec::new(),
        });
        assert_eq!(
            store.materialize_batch(&[oversized_name]),
            Err(UploadError::SelectionLimit)
        );
    }
    #[test]
    fn handles_are_opaque_and_revocable() {
        let store = UploadStore::default();
        let handle = store.insert(UploadFile {
            filename: "proof.txt".into(),
            media_type: "text/plain".into(),
            bytes: b"proof".to_vec(),
        });
        assert_eq!(store.descriptor(handle).unwrap().byte_length, 5);
        assert!(store.revoke(handle));
        assert_eq!(store.materialize(handle), Err(UploadError::UnknownHandle));
    }
}
