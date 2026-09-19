//! Parent-only persistent records for native service registrations and caches.
//! The profile owns the database; author storage RPC cannot select this table.
use super::*;
const RECORDS: TableDefinition<&str, &[u8]> = TableDefinition::new("native_service_records_v1");
const MAX_RECORD: usize = 64 * 1024 * 1024;
const MAX_NAMESPACE: usize = 128 * 1024 * 1024;
#[derive(Debug, Clone, Copy)]
pub enum ServiceStorageKind {
    Registrations,
    Caches,
}
#[derive(Debug, Clone)]
pub enum ServiceStorageRequest {
    Read {
        partition: String,
        origin: String,
        kind: ServiceStorageKind,
    },
    Replace {
        partition: String,
        origin: String,
        kind: ServiceStorageKind,
        expected_revision: u64,
        bytes: Vec<u8>,
    },
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceStorageSnapshot {
    pub revision: u64,
    pub bytes: Vec<u8>,
}
fn invalid(message: impl Into<String>) -> StorageError {
    StorageError::Persistent(message.into())
}
fn key(request: &ServiceStorageRequest) -> Result<(String, String), StorageError> {
    let (partition, origin, kind) = match request {
        ServiceStorageRequest::Read {
            partition,
            origin,
            kind,
        }
        | ServiceStorageRequest::Replace {
            partition,
            origin,
            kind,
            ..
        } => (partition, origin, kind),
    };
    if partition.is_empty()
        || origin.is_empty()
        || partition.len() > 8192
        || origin.len() > 8192
        || partition.contains('\0')
        || origin.contains('\0')
    {
        return Err(invalid("invalid native service storage authority"));
    }
    let prefix = format!("{partition}\0{origin}\0");
    Ok((
        format!(
            "{prefix}{}",
            match kind {
                ServiceStorageKind::Registrations => "registrations",
                ServiceStorageKind::Caches => "caches",
            }
        ),
        prefix,
    ))
}
fn replacement(
    current: &ServiceStorageSnapshot,
    request: ServiceStorageRequest,
    other_bytes: usize,
) -> Result<ServiceStorageSnapshot, StorageError> {
    match request {
        ServiceStorageRequest::Read { .. } => Ok(current.clone()),
        ServiceStorageRequest::Replace {
            expected_revision,
            bytes,
            origin,
            ..
        } => {
            if expected_revision != current.revision {
                return Err(StorageError::Constraint);
            }
            if bytes.len() > MAX_RECORD || other_bytes.saturating_add(bytes.len()) > MAX_NAMESPACE {
                return Err(StorageError::QuotaExceeded {
                    origin,
                    limit: MAX_NAMESPACE,
                });
            }
            Ok(ServiceStorageSnapshot {
                revision: current
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| invalid("native service storage revision exhausted"))?,
                bytes,
            })
        }
    }
}
pub(super) fn memory(
    records: &mut BTreeMap<String, ServiceStorageSnapshot>,
    request: ServiceStorageRequest,
) -> Result<ServiceStorageSnapshot, StorageError> {
    let (key, prefix) = key(&request)?;
    let current = records.get(&key).cloned().unwrap_or_default();
    let write = matches!(&request, ServiceStorageRequest::Replace { .. });
    let other = records
        .iter()
        .filter(|(name, _)| name.starts_with(&prefix) && **name != key)
        .fold(0usize, |sum, (_, record)| {
            sum.saturating_add(record.bytes.len())
        });
    let next = replacement(&current, request, other)?;
    if write {
        records.insert(key, next.clone());
    }
    Ok(next)
}
fn decode(bytes: &[u8]) -> Result<ServiceStorageSnapshot, StorageError> {
    if bytes.len() < 8 || bytes.len() > MAX_RECORD + 8 {
        return Err(invalid("invalid native service record length"));
    }
    let revision = u64::from_le_bytes(
        bytes[..8]
            .try_into()
            .map_err(|_| invalid("invalid native service revision"))?,
    );
    Ok(ServiceStorageSnapshot {
        revision,
        bytes: bytes[8..].to_vec(),
    })
}
pub(super) fn persistent(
    database: &Database,
    request: ServiceStorageRequest,
) -> Result<ServiceStorageSnapshot, StorageError> {
    let (key, prefix) = key(&request)?;
    // The write transaction serializes compare-and-replace across every context
    // sharing this profile, including first-read table creation on old profiles.
    let transaction = database
        .begin_write()
        .map_err(|error| invalid(error.to_string()))?;
    let next = {
        let mut table = transaction
            .open_table(RECORDS)
            .map_err(|error| invalid(error.to_string()))?;
        let current = table
            .get(key.as_str())
            .map_err(|error| invalid(error.to_string()))?
            .map(|bytes| decode(bytes.value()))
            .transpose()?
            .unwrap_or_default();
        let mut other = 0usize;
        for item in table.iter().map_err(|error| invalid(error.to_string()))? {
            let (name, value) = item.map_err(|error| invalid(error.to_string()))?;
            if name.value() != key && name.value().starts_with(&prefix) {
                let value = decode(value.value())?;
                other = other.saturating_add(value.bytes.len());
            }
        }
        let write = matches!(&request, ServiceStorageRequest::Replace { .. });
        let next = replacement(&current, request, other)?;
        if write {
            let mut encoded = Vec::with_capacity(8 + next.bytes.len());
            encoded.extend_from_slice(&next.revision.to_le_bytes());
            encoded.extend_from_slice(&next.bytes);
            table
                .insert(key.as_str(), encoded.as_slice())
                .map_err(|error| invalid(error.to_string()))?;
        }
        next
    };
    transaction
        .commit()
        .map_err(|error| invalid(error.to_string()))?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn write(partition: &str, revision: u64, bytes: &[u8]) -> ServiceStorageRequest {
        ServiceStorageRequest::Replace {
            partition: partition.into(),
            origin: "https://a.test".into(),
            kind: ServiceStorageKind::Caches,
            expected_revision: revision,
            bytes: bytes.into(),
        }
    }
    fn read(partition: &str) -> ServiceStorageRequest {
        ServiceStorageRequest::Read {
            partition: partition.into(),
            origin: "https://a.test".into(),
            kind: ServiceStorageKind::Caches,
        }
    }
    #[test]
    fn records_reject_lost_updates_and_isolate_native_partitions() {
        let store = SharedContextStore::memory(StorageLimits::default());
        assert_eq!(
            store
                .service_storage(write("https://top.test", 0, b"first"))
                .unwrap()
                .revision,
            1
        );
        assert!(matches!(
            store.service_storage(write("https://top.test", 0, b"lost")),
            Err(StorageError::Constraint)
        ));
        assert!(
            store
                .service_storage(read("http://top.test"))
                .unwrap()
                .bytes
                .is_empty()
        );
        assert_eq!(
            store
                .service_storage(read("https://top.test"))
                .unwrap()
                .bytes,
            b"first"
        );
        assert!(
            store
                .service_storage(write("invalid\0authority", 0, b"x"))
                .is_err()
        );
    }
    #[test]
    fn service_records_survive_profile_reopen_without_cross_profile_leak() {
        let directory = std::env::temp_dir().join(format!(
            "oryn-service-profile-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let profile = PersistentProfile::open(&directory).unwrap();
            let store = SharedContextStore::persistent(&profile, StorageLimits::default());
            store
                .service_storage(write("https://top.test", 0, b"durable"))
                .unwrap();
        }
        {
            let profile = PersistentProfile::open(&directory).unwrap();
            let store = SharedContextStore::persistent(&profile, StorageLimits::default());
            let record = store.service_storage(read("https://top.test")).unwrap();
            assert_eq!(record.revision, 1);
            assert_eq!(record.bytes, b"durable");
        }
        let other = directory.with_extension("other");
        {
            let profile = PersistentProfile::open(&other).unwrap();
            assert_eq!(
                SharedContextStore::persistent(&profile, StorageLimits::default())
                    .service_storage(read("https://top.test"))
                    .unwrap(),
                ServiceStorageSnapshot::default()
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
        std::fs::remove_dir_all(other).unwrap();
    }
}
