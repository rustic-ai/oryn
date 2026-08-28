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
        getrandom::fill(&mut random).expect("operating-system randomness is required");
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
        self.entries.lock().expect("download store lock").insert(
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
    #[error("download exceeds quarantine limit: {actual} > {limit} bytes")]
    TooLarge { limit: usize, actual: usize },
    #[error("download is unknown or was already released")]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

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
