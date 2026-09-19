//! Context-owned clipboard transport. No process-global or host OS clipboard.
use serde::{Deserialize, Serialize};
pub const MAX_CLIPBOARD_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_CLIPBOARD_ITEMS: usize = 256;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardPayload {
    Text(Vec<u16>),
    File {
        name: String,
        last_modified: i64,
        bytes: Vec<u8>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub media_type: String,
    pub payload: ClipboardPayload,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardData {
    pub items: Vec<ClipboardItem>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardError {
    NotAllowed,
    Retired,
    QuotaExceeded,
    Unavailable,
    InvalidData,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardAction {
    Copy,
    Cut,
    Paste,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardActionResult {
    pub event_cancelled: bool,
    pub clipboard_changed: bool,
    pub content_changed: bool,
}
pub trait ClipboardBackend: Send + Sync {
    /// Native transports override these receiver-stamped operations. Defaults
    /// retain standalone/in-memory backend compatibility without inventing IPC authority.
    fn authorize_script_for(
        &self,
        _creator: &crate::worker_wire::Creator,
        origins: &[String],
        write: bool,
    ) -> Result<(), ClipboardError> {
        self.authorize_script(origins, write)
    }
    fn read_for(
        &self,
        _creator: &crate::worker_wire::Creator,
    ) -> Result<ClipboardData, ClipboardError> {
        self.read()
    }
    fn write_for(
        &self,
        _creator: &crate::worker_wire::Creator,
        value: ClipboardData,
    ) -> Result<(), ClipboardError> {
        self.write(value)
    }

    /// Native Window ancestry only; implementations must enforce active page and policy.
    fn authorize_script(&self, _origins: &[String], _write: bool) -> Result<(), ClipboardError> {
        Err(ClipboardError::NotAllowed)
    }
    fn read(&self) -> Result<ClipboardData, ClipboardError>;
    /// An error does not guarantee that no write occurred (transport replies
    /// can be lost). Callers must not replay a failed write as the same action.
    fn write(&self, value: ClipboardData) -> Result<(), ClipboardError>;
}
impl ClipboardData {
    pub fn text(value: Vec<u16>) -> Self {
        Self {
            items: vec![ClipboardItem {
                media_type: "text/plain".into(),
                payload: ClipboardPayload::Text(value),
            }],
        }
    }
    pub fn plain_text(&self) -> &[u16] {
        self.items
            .iter()
            .find_map(|item| {
                if item.media_type == "text/plain" {
                    if let ClipboardPayload::Text(value) = &item.payload {
                        Some(value.as_slice())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or(&[])
    }
    pub fn validate(&self) -> Result<(), ClipboardError> {
        if self.items.len() > MAX_CLIPBOARD_ITEMS {
            return Err(ClipboardError::QuotaExceeded);
        }
        let mut size = 0usize;
        let mut text_types = std::collections::BTreeSet::new();
        for item in &self.items {
            if item.media_type.is_empty()
                || item
                    .media_type
                    .bytes()
                    .any(|byte| byte.is_ascii_uppercase() || byte.is_ascii_control())
            {
                return Err(ClipboardError::InvalidData);
            }
            let payload = match &item.payload {
                ClipboardPayload::Text(value) => {
                    if !text_types.insert(&item.media_type) {
                        return Err(ClipboardError::InvalidData);
                    }
                    value
                        .len()
                        .checked_mul(2)
                        .ok_or(ClipboardError::QuotaExceeded)?
                }
                ClipboardPayload::File { name, bytes, .. } => {
                    name.len()
                        .checked_add(bytes.len())
                        .ok_or(ClipboardError::QuotaExceeded)?
                }
            };
            size = size
                .checked_add(item.media_type.len())
                .and_then(|size| size.checked_add(payload))
                .ok_or(ClipboardError::QuotaExceeded)?;
            if size > MAX_CLIPBOARD_BYTES {
                return Err(ClipboardError::QuotaExceeded);
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_preserves_units_and_rejects_ambiguous_or_oversized_payload() {
        let data = ClipboardData::text(vec![0xd800, 97]);
        assert_eq!(data.plain_text(), &[0xd800, 97]);
        assert!(data.validate().is_ok());
        let mut duplicate = data.clone();
        duplicate.items.extend(data.items.clone());
        assert_eq!(duplicate.validate(), Err(ClipboardError::InvalidData));
        let oversized = ClipboardData::text(vec![0; MAX_CLIPBOARD_BYTES / 2 + 1]);
        assert_eq!(oversized.validate(), Err(ClipboardError::QuotaExceeded));
    }
}
