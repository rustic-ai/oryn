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
        self.entries
            .lock()
            .expect("upload store lock")
            .get(&handle)
            .map(|entry| entry.file.clone())
            .ok_or(UploadError::UnknownHandle)
    }

    pub fn descriptor(&self, handle: UploadHandle) -> Result<UploadDescriptor, UploadError> {
        let file = self.materialize(handle)?;
        Ok(UploadDescriptor {
            filename: file.filename,
            media_type: file.media_type,
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
    #[error("upload handle is unknown or revoked")]
    UnknownHandle,
    #[error("upload label is unknown")]
    UnknownLabel,
}

#[cfg(test)]
mod tests {
    use super::*;

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
