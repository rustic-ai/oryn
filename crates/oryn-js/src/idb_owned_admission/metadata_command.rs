//! Exact copied string/container ownership for existing native IDB DTOs.
//! JSON values retain the existing allocation policy; no JSON heap closure claim.
use super::{Buffer, Lease, Ledger, Text};
use crate::platform::{
    IdbHostCommand as C, IdbIndexDefinition, IdbObjectStoreDefinition, IdbWrite,
};
use std::sync::Arc;

pub struct OwnedMetadataCommand {
    command: C,
    custody: Buffer<Lease>,
}
impl OwnedMetadataCommand {
    pub fn copy(
        command: &C,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        let mut count = 0usize;
        count_command(command, &mut count, &mut cancelled)?;
        let mut custody = Buffer::allocate(ledger, count, limit, used)?;
        let command = copy_command(command, ledger, limit, used, &mut custody, &mut cancelled)?;
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        Ok(Self { command, custody })
    }
    pub fn into_parts(self) -> (C, Buffer<Lease>) {
        (self.command, self.custody)
    }
    pub fn command(&self) -> &C {
        &self.command
    }
}
fn add(n: &mut usize, k: usize) -> Result<(), &'static str> {
    *n = n.checked_add(k).ok_or("IDB metadata count")?;
    Ok(())
}
fn count_command(
    mut c: &C,
    n: &mut usize,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<(), &'static str> {
    loop {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        match c {
            C::UpgradeConnection { inner, .. } | C::Transaction { inner, .. } => {
                add(n, 3)?;
                c = inner;
            }
            _ => break,
        }
    }
    match c {
        C::UpgradeConnection { .. } | C::Transaction { .. } => unreachable!(),
        C::BeginTransaction {
            scope, connection, ..
        } => {
            add(n, 4)?;
            add(n, scope.len())?;
            add(n, usize::from(connection.is_some()))
        }
        C::CreateObjectStore { definition, .. } => {
            add(n, 3)?;
            add(n, usize::from(definition.key_path.is_some()))?;
            add(
                n,
                definition
                    .indexes
                    .len()
                    .checked_mul(2)
                    .ok_or("IDB metadata count")?,
            )
        }
        C::CreateIndex { .. } | C::RenameIndex { .. } => add(n, 4),
        C::Batch { writes, .. } => {
            add(n, 2)?;
            add(n, writes.len())
        }
        C::Cursor { index, .. } | C::CursorStep { index, .. } => {
            add(n, 2)?;
            add(n, usize::from(index.is_some()))
        }
        C::DeleteIndex { .. } | C::RenameObjectStore { .. } => add(n, 3),
        C::ListDatabases => Ok(()),
        C::Open { .. }
        | C::DeleteDatabase { .. }
        | C::AbortUpgrade { .. }
        | C::CommitUpgrade { .. } => add(n, 1),
        _ => add(n, 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_empty_and_short_final_chunk_release() {
        let ledger = Arc::new(Ledger::default());
        assert!(OwnedMetadataCommand::copy(&C::ListDatabases, &ledger, 4096, 0, || true).is_err());
        assert_eq!(ledger.live_bytes(), 0);
        let command = C::CommitUpgrade {
            name: String::new(),
        };
        let mut calls = 0;
        assert!(
            OwnedMetadataCommand::copy(&command, &ledger, 4096, 0, || {
                calls += 1;
                calls >= 4
            })
            .is_err()
        );
        assert_eq!(ledger.live_bytes(), 0);
        let command = C::CommitUpgrade { name: "x".into() };
        let mut calls = 0;
        assert!(
            OwnedMetadataCommand::copy(&command, &ledger, 4096, 0, || {
                calls += 1;
                calls >= 5
            })
            .is_err()
        );
        assert_eq!(calls, 5);
        assert_eq!(ledger.live_bytes(), 0);
    }
    #[test]
    fn recursive_metadata_attempts_preserve_wire_and_release_exactly() {
        let ledger = Arc::new(Ledger::default());
        let command = C::UpgradeConnection {
            token: "token".into(),
            name: "database".into(),
            inner: Box::new(C::AbortUpgrade {
                name: "database".into(),
                old_version: 1,
            }),
        };
        let original = serde_json::to_value(&command).unwrap();
        let retained = OwnedMetadataCommand::copy(&command, &ledger, 1 << 20, 0, || false).unwrap();
        let baseline = ledger.live_bytes();
        let attempt =
            OwnedMetadataCommand::copy(retained.command(), &ledger, 1 << 20, 0, || false).unwrap();
        assert_eq!(ledger.live_bytes(), baseline * 2);
        let (command, custody) = attempt.into_parts();
        assert_eq!(serde_json::to_value(&command).unwrap(), original);
        drop(command);
        assert_eq!(ledger.live_bytes(), baseline * 2);
        drop(custody);
        assert_eq!(ledger.live_bytes(), baseline);
        assert!(
            OwnedMetadataCommand::copy(retained.command(), &ledger, baseline, 0, || false).is_err()
        );
        assert_eq!(ledger.live_bytes(), baseline);
        drop(retained);
        assert_eq!(ledger.live_bytes(), 0);
    }
}
fn text(
    s: &str,
    l: &Arc<Ledger>,
    limit: usize,
    used: usize,
    c: &mut Buffer<Lease>,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<String, &'static str> {
    if cancelled() {
        return Err("IDB metadata cancelled");
    }
    let mut bytes = Buffer::allocate(l, s.len(), limit, used)?;
    for (offset, byte) in s.bytes().enumerate() {
        if offset % 1024 == 0 && cancelled() {
            return Err("IDB metadata cancelled");
        }
        bytes.push(byte)?;
    }
    let Text { value, lease } = Text::from_admitted_utf8(bytes)?;
    if cancelled() {
        return Err("IDB metadata cancelled");
    }
    c.push(lease)?;
    Ok(value)
}
fn boxed(
    value: C,
    l: &Arc<Ledger>,
    limit: usize,
    used: usize,
    c: &mut Buffer<Lease>,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<Box<C>, &'static str> {
    if cancelled() {
        return Err("IDB metadata cancelled");
    }
    let mut b = Buffer::allocate(l, 1, limit, used)?;
    b.push(value)?;
    let Buffer { values, lease } = b;
    c.push(lease)?;
    let mut values = std::mem::ManuallyDrop::new(values);
    // Exact one-element layout equals Box<C>; consumed without reallocation.
    Ok(unsafe { Box::from_raw(values.as_mut_ptr()) })
}
fn copy_command(
    c: &C,
    l: &Arc<Ledger>,
    limit: usize,
    used: usize,
    leases: &mut Buffer<Lease>,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<C, &'static str> {
    let mut leaf = c;
    let mut depth = 0usize;
    while let C::UpgradeConnection { inner, .. } | C::Transaction { inner, .. } = leaf {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        depth = depth.checked_add(1).ok_or("IDB metadata depth")?;
        leaf = inner;
    }
    let mut wrappers = Buffer::allocate(l, depth, limit, used)?;
    let mut current = c;
    while let C::UpgradeConnection { inner, .. } | C::Transaction { inner, .. } = current {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        wrappers.push(current)?;
        current = inner;
    }
    let mut command = copy_leaf(leaf, l, limit, used, leases, cancelled)?;
    for wrapper in wrappers.as_slice().iter().rev() {
        if cancelled() {
            return Err("IDB metadata cancelled");
        }
        command = match wrapper {
            C::UpgradeConnection { token, name, .. } => C::UpgradeConnection {
                token: text(token, l, limit, used, leases, cancelled)?,
                name: text(name, l, limit, used, leases, cancelled)?,
                inner: boxed(command, l, limit, used, leases, cancelled)?,
            },
            C::Transaction {
                token, database, ..
            } => C::Transaction {
                token: text(token, l, limit, used, leases, cancelled)?,
                database: text(database, l, limit, used, leases, cancelled)?,
                inner: boxed(command, l, limit, used, leases, cancelled)?,
            },
            _ => unreachable!(),
        };
    }
    Ok(command)
}
fn copy_leaf(
    c: &C,
    l: &Arc<Ledger>,
    limit: usize,
    used: usize,
    leases: &mut Buffer<Lease>,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<C, &'static str> {
    macro_rules! s {
        ($x:expr) => {
            text($x, l, limit, used, leases, cancelled)?
        };
    }
    Ok(match c {
        C::BeginTransaction {
            token,
            database,
            scope,
            mode,
            connection,
        } => {
            let mut names = Buffer::allocate(l, scope.len(), limit, used)?;
            for name in scope {
                if cancelled() {
                    return Err("IDB metadata cancelled");
                }
                names.push(s!(name))?
            }
            let Buffer {
                values: scope,
                lease,
            } = names;
            leases.push(lease)?;
            C::BeginTransaction {
                token: s!(token),
                database: s!(database),
                scope,
                mode: s!(mode),
                connection: connection
                    .as_deref()
                    .map(|v| text(v, l, limit, used, leases, cancelled))
                    .transpose()?,
            }
        }
        C::UpgradeConnection { token, name, inner } => {
            let inner = copy_command(inner, l, limit, used, leases, cancelled)?;
            C::UpgradeConnection {
                token: s!(token),
                name: s!(name),
                inner: boxed(inner, l, limit, used, leases, cancelled)?,
            }
        }
        C::Transaction {
            token,
            database,
            inner,
        } => {
            let inner = copy_command(inner, l, limit, used, leases, cancelled)?;
            C::Transaction {
                token: s!(token),
                database: s!(database),
                inner: boxed(inner, l, limit, used, leases, cancelled)?,
            }
        }
        C::OpenConnection {
            token,
            name,
            version,
        } => C::OpenConnection {
            token: s!(token),
            name: s!(name),
            version: *version,
        },
        C::DeleteConnection { token, name } => C::DeleteConnection {
            token: s!(token),
            name: s!(name),
        },
        C::CloseConnection { token, name } => C::CloseConnection {
            token: s!(token),
            name: s!(name),
        },
        C::RetireConnection { token, name } => C::RetireConnection {
            token: s!(token),
            name: s!(name),
        },
        C::PollConnection { token, name } => C::PollConnection {
            token: s!(token),
            name: s!(name),
        },
        C::AcknowledgeConnection { token, name, event } => C::AcknowledgeConnection {
            token: s!(token),
            name: s!(name),
            event: *event,
        },
        C::CommitTransaction { token, database } => C::CommitTransaction {
            token: s!(token),
            database: s!(database),
        },
        C::AbortTransaction { token, database } => C::AbortTransaction {
            token: s!(token),
            database: s!(database),
        },
        C::ListDatabases => C::ListDatabases,
        C::Open { name, version } => C::Open {
            name: s!(name),
            version: *version,
        },
        C::DeleteDatabase { name } => C::DeleteDatabase { name: s!(name) },
        C::AbortUpgrade { name, old_version } => C::AbortUpgrade {
            name: s!(name),
            old_version: *old_version,
        },
        C::CommitUpgrade { name } => C::CommitUpgrade { name: s!(name) },
        C::CreateObjectStore {
            database,
            definition,
        } => {
            let mut indexes = Buffer::allocate(l, definition.indexes.len(), limit, used)?;
            for index in &definition.indexes {
                if cancelled() {
                    return Err("IDB metadata cancelled");
                }
                indexes.push(IdbIndexDefinition {
                    name: s!(&index.name),
                    key_path: s!(&index.key_path),
                    unique: index.unique,
                    multi_entry: index.multi_entry,
                })?
            }
            let Buffer {
                values: indexes,
                lease,
            } = indexes;
            leases.push(lease)?;
            C::CreateObjectStore {
                database: s!(database),
                definition: IdbObjectStoreDefinition {
                    name: s!(&definition.name),
                    key_path: definition
                        .key_path
                        .as_deref()
                        .map(|v| text(v, l, limit, used, leases, cancelled))
                        .transpose()?,
                    auto_increment: definition.auto_increment,
                    indexes,
                },
            }
        }
        C::CreateIndex {
            database,
            store,
            definition,
        } => C::CreateIndex {
            database: s!(database),
            store: s!(store),
            definition: IdbIndexDefinition {
                name: s!(&definition.name),
                key_path: s!(&definition.key_path),
                unique: definition.unique,
                multi_entry: definition.multi_entry,
            },
        },
        C::DeleteObjectStore { database, store } => C::DeleteObjectStore {
            database: s!(database),
            store: s!(store),
        },
        C::DeleteIndex {
            database,
            store,
            index,
        } => C::DeleteIndex {
            database: s!(database),
            store: s!(store),
            index: s!(index),
        },
        C::RenameObjectStore {
            database,
            store,
            new_name,
        } => C::RenameObjectStore {
            database: s!(database),
            store: s!(store),
            new_name: s!(new_name),
        },
        C::RenameIndex {
            database,
            store,
            index,
            new_name,
        } => C::RenameIndex {
            database: s!(database),
            store: s!(store),
            index: s!(index),
            new_name: s!(new_name),
        },
        C::Batch { database, writes } => {
            let mut copy = Buffer::allocate(l, writes.len(), limit, used)?;
            for w in writes {
                if cancelled() {
                    return Err("IDB metadata cancelled");
                }
                copy.push(IdbWrite {
                    store: s!(&w.store),
                    key: w.key.clone(),
                    value: w.value.clone(),
                    add_only: w.add_only,
                })?
            }
            let Buffer {
                values: writes,
                lease,
            } = copy;
            leases.push(lease)?;
            C::Batch {
                database: s!(database),
                writes,
            }
        }
        C::Get {
            database,
            store,
            key,
        } => C::Get {
            database: s!(database),
            store: s!(store),
            key: key.clone(),
        },
        C::GetAll { database, store } => C::GetAll {
            database: s!(database),
            store: s!(store),
        },
        C::Delete {
            database,
            store,
            key,
        } => C::Delete {
            database: s!(database),
            store: s!(store),
            key: key.clone(),
        },
        C::Clear { database, store } => C::Clear {
            database: s!(database),
            store: s!(store),
        },
        C::Count { database, store } => C::Count {
            database: s!(database),
            store: s!(store),
        },
        C::Cursor {
            database,
            store,
            index,
            query,
            reverse,
        } => C::Cursor {
            database: s!(database),
            store: s!(store),
            index: index
                .as_deref()
                .map(|v| text(v, l, limit, used, leases, cancelled))
                .transpose()?,
            query: query.clone(),
            reverse: *reverse,
        },
        C::CursorStep {
            database,
            store,
            index,
            query,
            reverse,
            step,
        } => C::CursorStep {
            database: s!(database),
            store: s!(store),
            index: index
                .as_deref()
                .map(|v| text(v, l, limit, used, leases, cancelled))
                .transpose()?,
            query: query.clone(),
            reverse: *reverse,
            step: step.clone(),
        },
    })
}
