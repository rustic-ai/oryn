//! Internal thread-safe ownership and accounting for IndexedDB admission payloads.
use std::sync::{Arc, Mutex};
#[path = "idb_owned_admission/metadata_command.rs"]
mod metadata_command;

#[derive(Default)]
pub struct Ledger(Mutex<State>);
#[derive(Default)]
struct State {
    live: usize,
    accounted: usize,
    delta: i64,
    flushing: bool,
}
/// Snapshot separates non-ledger heap from mutable admitted native ownership.
#[derive(Clone)]
pub(crate) struct HeapReceipt {
    limit: usize,
    other: usize,
    used_heap: usize,
    backing_store: usize,
    ledger: std::sync::Weak<Ledger>,
}
pub struct Adjustment {
    amount: i64,
    ledger: Arc<Ledger>,
    finished: bool,
}
pub struct Lease {
    ledger: Arc<Ledger>,
    bytes: usize,
}
impl Ledger {
    pub fn live_bytes(&self) -> usize {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).live
    }
    pub(crate) fn heap_receipt(self: &Arc<Self>, limit: usize, used: usize) -> Result<HeapReceipt, String> {
        // V8 150 HeapStatistics::external_memory reports backing-store bytes,
        // not the embedder counter adjusted for GC pressure. `used` therefore
        // excludes our admitted native allocations; include live custody once.
        Ok(HeapReceipt { limit, other: used, used_heap: used, backing_store: 0, ledger: Arc::downgrade(self) })
    }
    pub(crate) fn heap_receipt_with_statistics(self: &Arc<Self>, limit: usize,
        used_heap: usize, backing_store: usize) -> Result<HeapReceipt, String> {
        let other=used_heap.checked_add(backing_store).ok_or("native retained heap accounting")?;
        Ok(HeapReceipt { limit, other, used_heap, backing_store, ledger: Arc::downgrade(self) })
    }
    pub(crate) fn reserve_from_receipt(self: &Arc<Self>, bytes: usize, receipt: &HeapReceipt) -> Result<Lease, String> {
        if !std::ptr::eq(self.as_ref(), receipt.ledger.as_ptr()) {
            return Err("IDB allocation receipt owner mismatch".into());
        }
        let mut state = self.0.lock().map_err(|_| "IDB allocation ledger poisoned")?;
        let live = state.live.checked_add(bytes).ok_or("IDB allocation overflow")?;
        if receipt.other.checked_add(live).is_none_or(|n| n > receipt.limit) {
            retained_limit_diagnostic(bytes,state.live,receipt.used_heap,receipt.backing_store,receipt.limit);
            return Err("IDB allocation limit".into());
        }
        let delta = state.delta.checked_add(i64::try_from(bytes).map_err(|_| "IDB allocation overflow")?)
            .ok_or("IDB allocation overflow")?;
        state.live = live;
        state.delta = delta;
        Ok(Lease { ledger: self.clone(), bytes })
    }
    pub fn reserve(
        self: &Arc<Self>,
        bytes: usize,
        limit: usize,
        used: usize,
    ) -> Result<Lease, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "IDB allocation ledger poisoned")?;
        let other = used;
        let live = state
            .live
            .checked_add(bytes)
            .ok_or("IDB allocation overflow")?;
        if other.checked_add(live).is_none_or(|n| n > limit) {
            return Err("IDB allocation limit".into());
        }
        let delta = state
            .delta
            .checked_add(i64::try_from(bytes).map_err(|_| "IDB allocation overflow")?)
            .ok_or("IDB allocation overflow")?;
        state.live = live;
        state.delta = delta;
        Ok(Lease {
            ledger: self.clone(),
            bytes,
        })
    }
    pub fn begin_adjustment(self: &Arc<Self>) -> Result<Option<Adjustment>, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "IDB allocation ledger poisoned")?;
        if state.flushing || state.delta == 0 {
            return Ok(None);
        }
        state.flushing = true;
        Ok(Some(Adjustment {
            amount: std::mem::take(&mut state.delta),
            ledger: self.clone(),
            finished: false,
        }))
    }
    pub fn finish_adjustment(&self, mut adjustment: Adjustment) -> Result<(), String> {
        if !std::ptr::eq(self, Arc::as_ptr(&adjustment.ledger)) {
            return Err("IDB allocation receipt owner mismatch".into());
        }
        let mut state = self
            .0
            .lock()
            .map_err(|_| "IDB allocation ledger poisoned")?;
        if !state.flushing {
            return Err("IDB allocation adjustment missing".into());
        }
        state.accounted = if adjustment.amount >= 0 {
            state.accounted.checked_add(adjustment.amount as usize)
        } else {
            state
                .accounted
                .checked_sub(adjustment.amount.unsigned_abs() as usize)
        }
        .ok_or("IDB allocation adjustment overflow")?;
        state.flushing = false;
        adjustment.finished = true;
        Ok(())
    }
}
impl Adjustment {
    pub fn amount(&self) -> i64 {
        self.amount
    }
}
impl Drop for Adjustment {
    fn drop(&mut self) {
        if !self.finished {
            let mut state = self.ledger.0.lock().unwrap_or_else(|e| e.into_inner());
            state.delta = state
                .delta
                .checked_add(self.amount)
                .expect("IDB adjustment invariant");
            state.flushing = false;
        }
    }
}

// Developer-only, numeric failure diagnostics. Disabled unless explicitly opted in;
// no document/URL/database/value/token text and no changed admission result.
fn retained_limit_diagnostic(requested:usize, live:usize, used_heap:usize,
    backing_store:usize, limit:usize) {
    static EMITTED:std::sync::atomic::AtomicUsize=std::sync::atomic::AtomicUsize::new(0);
    if std::env::var("ORYN_NATIVE_ALLOCATION_DIAGNOSTICS").ok().as_deref()!=Some("1") {return;}
    if EMITTED.fetch_update(std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,|n|(n<16).then_some(n+1)).is_err() {return;}
    eprintln!("oryn-native-retained-limit requested={requested} live={live} used_heap={used_heap} backing_store={backing_store} limit={limit}");
}

#[cfg(test)]
mod retained_limit_statistics_tests {
    use super::*;
    #[test]
    fn numeric_statistics_receipt_preserves_original_limit_and_lease_atomicity() {
        let ledger=Arc::new(Ledger::default());
        let receipt=ledger.heap_receipt_with_statistics(100,10,20).unwrap();
        assert_eq!((receipt.used_heap,receipt.backing_store,receipt.other),(10,20,30));
        let lease=ledger.reserve_from_receipt(70,&receipt).unwrap();
        assert_eq!(ledger.reserve_from_receipt(1,&receipt).err().as_deref(),Some("IDB allocation limit"));
        assert_eq!(ledger.live_bytes(),70);drop(lease);assert_eq!(ledger.live_bytes(),0);
        assert!(ledger.heap_receipt_with_statistics(usize::MAX,usize::MAX,1).is_err());
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        // Drop cannot abandon accounting on a poisoned lock; recovery owns only
        // numeric custody, never author state or parent authority.
        let mut state = self
            .ledger
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.live = state
            .live
            .checked_sub(self.bytes)
            .expect("IDB live custody invariant");
        state.delta = state
            .delta
            .checked_sub(self.bytes as i64)
            .expect("IDB delta custody invariant");
    }
}

use std::alloc::{Layout, alloc};
/// Owned exact-capacity vector, with its allocation lease moved alongside it.
/// No allocation registry/map; dropping cancels/releases native ownership.
pub struct Buffer<T> {
    values: Vec<T>,
    lease: Lease,
}
impl<T> Buffer<T> {
    pub(crate) fn into_values(self)->AdmittedValues<T>{let Buffer{values,lease}=self;AdmittedValues{values:values.into_iter(),_lease:lease}}

    pub fn as_slice(&self) -> &[T] {
        &self.values
    }
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.values
    }
    pub fn allocate(
        ledger: &Arc<Ledger>,
        len: usize,
        limit: usize,
        used: usize,
    ) -> Result<Self, &'static str> {
        if std::mem::size_of::<T>() == 0 {
            return Err("IDB zero-sized payload unsupported");
        }
        let layout = Layout::array::<T>(len).map_err(|_| "IDB allocation layout")?;
        let lease = ledger
            .reserve(layout.size(), limit, used)
            .map_err(|_| "IDB allocation admission")?;
        Self::allocate_with_lease(layout, len, lease)
    }
    fn allocate_from_receipt(ledger: &Arc<Ledger>, len: usize, receipt: &HeapReceipt) -> Result<Self, &'static str> {
        if std::mem::size_of::<T>() == 0 { return Err("IDB zero-sized payload unsupported"); }
        let layout = Layout::array::<T>(len).map_err(|_| "IDB allocation layout")?;
        let lease = ledger.reserve_from_receipt(layout.size(), receipt).map_err(|_| "IDB allocation admission")?;
        Self::allocate_with_lease(layout, len, lease)
    }
    fn allocate_with_lease(layout: Layout, len: usize, lease: Lease) -> Result<Self, &'static str> {
        let values = if layout.size() == 0 {
            Vec::new()
        } else {
            // SAFETY: allocator allocation uses the exact Vec element layout;
            // length starts at zero, capacity exactly len, and Vec later drops
            // initialized items and deallocates with this same layout. A null
            // allocation returns an error rather than aborting. No raw V8 scope.
            let pointer = unsafe { alloc(layout) };
            if pointer.is_null() {
                return Err("IDB allocation failed");
            }
            unsafe { Vec::from_raw_parts(pointer.cast::<T>(), 0, len) }
        };
        Ok(Self { values, lease })
    }
    pub fn push(&mut self, value: T) -> Result<(), &'static str> {
        if self.values.len() == self.values.capacity() {
            return Err("IDB exact allocation exhausted");
        }
        self.values.push(value);
        Ok(())
    }
}

pub(crate) struct AdmittedValues<T>{values:std::vec::IntoIter<T>,_lease:Lease}
impl<T> Iterator for AdmittedValues<T>{type Item=T;fn next(&mut self)->Option<T>{self.values.next()}}

pub struct Text {
    value: String,
    lease: Lease,
}
impl Buffer<u16> {
    pub fn initialize_zeroed(&mut self) {
        while self.values.len() < self.values.capacity() {
            self.values.push(0)
        }
    }
}
impl Text {
    pub(crate) fn from_admitted_utf8(buffer:Buffer<u8>)->Result<Self,&'static str>{let Buffer{values,lease}=buffer;let value=String::from_utf8(values).map_err(|_|"IDB metadata UTF8")?;Ok(Self{value,lease})}

    pub(crate) fn into_parts(self)->(String,Lease) {(self.value,self.lease)}

    pub fn copy_string(
        ledger: &Arc<Ledger>,
        value: &str,
        limit: usize,
        used: usize,
    ) -> Result<Self, &'static str> {
        Self::copy_string_cancelled(ledger, value, limit, used, || false)
    }
    pub(crate) fn copy_string_from_receipt(ledger: &Arc<Ledger>, value: &str, receipt: &HeapReceipt, mut cancelled: impl FnMut() -> bool) -> Result<Self, &'static str> {
        if cancelled() { return Err("IDB text cancelled"); }
        let mut bytes = Buffer::<u8>::allocate_from_receipt(ledger, value.len(), receipt)?;
        for (offset, byte) in value.bytes().enumerate() {
            if offset.is_multiple_of(1024) && cancelled() { return Err("IDB text cancelled"); }
            bytes.push(byte)?;
        }
        if cancelled() { return Err("IDB text cancelled"); }
        Self::from_admitted_utf8(bytes)
    }
    pub fn copy_string_cancelled(
        ledger: &Arc<Ledger>,
        value: &str,
        limit: usize,
        used: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB text cancelled");
        }
        let mut bytes = Buffer::<u8>::allocate(ledger, value.len(), limit, used)?;
        for (offset, byte) in value.bytes().enumerate() {
            if offset.is_multiple_of(1024) && cancelled() {
                return Err("IDB text cancelled");
            }
            bytes.push(byte)?
        }
        if cancelled() {
            return Err("IDB text cancelled");
        }
        let Buffer { values, lease } = bytes;
        Ok(Self {
            value: String::from_utf8(values).map_err(|_| "IDB metadata UTF8")?,
            lease,
        })
    }
    pub fn as_str(&self) -> &str {
        &self.value
    }
    pub fn capacity(&self) -> usize {
        self.value.capacity()
    }
    /// Encode the existing lossless DOMString representation without temporary
    /// per-unit strings or allocator over-reservation. The admitted allocation
    /// is retained alongside the resulting String, even when logical length is
    /// less than capacity. Invalid surrogate sequences use decimal UTF16 units.
    pub fn from_units(
        ledger: &Arc<Ledger>,
        units: &[u16],
        limit: usize,
        used: usize,
    ) -> Result<Self, &'static str> {
        Self::from_units_cancelled(ledger, units, limit, used, || false)
    }
    pub fn from_units_cancelled(
        ledger: &Arc<Ledger>,
        units: &[u16],
        limit: usize,
        used: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB text cancelled");
        }
        let mut valid = true;
        let mut unicode_bytes = 0usize;
        for (index, scalar) in char::decode_utf16(units.iter().copied()).enumerate() {
            if index % 1024 == 0 && cancelled() {
                return Err("IDB text cancelled");
            }
            match scalar {
                Ok(ch) => {
                    unicode_bytes = unicode_bytes
                        .checked_add(ch.len_utf8())
                        .ok_or("IDB text overflow")?
                }
                Err(_) => {
                    valid = false;
                    break;
                }
            }
        }
        // Prefix detection avoids an ordinary Unicode value colliding with the
        // DOM's reserved storage representation. It allocates no string.
        let prefix = oryn_dom::dom_string::PREFIX;
        let starts_prefix = units
            .iter()
            .copied()
            .take(prefix.encode_utf16().count())
            .eq(prefix.encode_utf16());
        let encoded = !valid || starts_prefix;
        let capacity = if encoded {
            units
                .len()
                .checked_mul(6)
                .and_then(|n| n.checked_add(prefix.len() + 2))
                .ok_or("IDB text overflow")?
        } else {
            unicode_bytes
        };
        if cancelled() {
            return Err("IDB text cancelled");
        }
        let mut buffer = Buffer::<u8>::allocate(ledger, capacity, limit, used)?;
        if encoded {
            for byte in prefix.bytes().chain(std::iter::once(b'[')) {
                buffer.push(byte)?
            }
            for (index, unit) in units.iter().copied().enumerate() {
                if index % 1024 == 0 && cancelled() {
                    return Err("IDB text cancelled");
                }
                if index != 0 {
                    buffer.push(b',')?
                }
                let mut digits = [0u8; 5];
                let mut count = 0;
                let mut number = unit;
                loop {
                    digits[count] = b'0' + (number % 10) as u8;
                    count += 1;
                    number /= 10;
                    if number == 0 {
                        break;
                    }
                }
                for digit in digits[..count].iter().rev() {
                    buffer.push(*digit)?
                }
            }
            buffer.push(b']')?;
        } else {
            for (index, scalar) in char::decode_utf16(units.iter().copied()).enumerate() {
                if index % 1024 == 0 && cancelled() {
                    return Err("IDB text cancelled");
                }
                let scalar = scalar.map_err(|_| "IDB UTF16 changed")?;
                let mut bytes = [0u8; 4];
                for byte in scalar.encode_utf8(&mut bytes).bytes() {
                    buffer.push(byte)?
                }
            }
        }
        if cancelled() {
            return Err("IDB text cancelled");
        }
        let Buffer { values, lease } = buffer;
        let value = String::from_utf8(values).map_err(|_| "IDB UTF8 encoding")?;
        Ok(Self { value, lease })
    }
}

/// Immutable native scope plus paired custody; no clone implementation permits
/// an unaccounted payload copy. Each command retry obtains a fresh admitted copy.
pub struct Scope {
    names: Buffer<String>,
    strings: Buffer<Lease>,
}
pub struct ScopeCustody {
    _vector: Lease,
    _strings: Buffer<Lease>,
    metadata: Option<(Lease, Lease, Lease, Option<Lease>)>,
}
impl Scope {
    pub fn copy_names(
        ledger: &Arc<Ledger>,
        names: &[String],
        limit: usize,
        used: usize,
    ) -> Result<Self, &'static str> {
        Self::copy_names_cancelled(ledger, names, limit, used, || false)
    }
    pub fn copy_names_cancelled(
        ledger: &Arc<Ledger>,
        names: &[String],
        limit: usize,
        used: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB scope cancelled");
        }
        let mut copy = Self::allocate(ledger, names.len(), limit, used)?;
        for name in names {
            copy.push(Text::copy_string_cancelled(
                ledger,
                name,
                limit,
                used,
                &mut cancelled,
            )?)?;
        }
        if cancelled() {
            return Err("IDB scope cancelled");
        }
        Ok(copy)
    }
    pub fn normalize(self) -> Result<Self, &'static str> {
        self.normalize_cancelled(|| false)
    }
    pub fn normalize_cancelled(
        mut self,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        fn sift(
            v: &mut [String],
            mut root: usize,
            end: usize,
            c: &mut dyn FnMut() -> bool,
        ) -> Result<(), &'static str> {
            loop {
                if c() {
                    return Err("IDB scope cancelled");
                }
                let Some(left) = root.checked_mul(2).and_then(|x| x.checked_add(1)) else {
                    return Ok(());
                };
                if left >= end {
                    return Ok(());
                }
                let right = left + 1;
                let child =
                    if right < end && compare_names_cancelled(&v[left], &v[right], c)?.is_lt() {
                        right
                    } else {
                        left
                    };
                if !compare_names_cancelled(&v[root], &v[child], c)?.is_lt() {
                    return Ok(());
                }
                v.swap(root, child);
                root = child;
            }
        }
        if cancelled() {
            return Err("IDB scope cancelled");
        }
        // Sorting borrows stored strings and never allocates per-comparison
        // UTF16 key vectors. Decimal reserved representations decode lazily.
        let values = &mut self.names.values;
        let length = values.len();
        for root in (0..length / 2).rev() {
            sift(values, root, length, &mut cancelled)?;
        }
        for end in (1..values.len()).rev() {
            if cancelled() {
                return Err("IDB scope cancelled");
            }
            values.swap(0, end);
            sift(values, 0, end, &mut cancelled)?;
        }
        let mut write = 0usize;
        for read in 0..values.len() {
            if cancelled() {
                return Err("IDB scope cancelled");
            }
            if write == 0
                || !compare_names_cancelled(&values[write - 1], &values[read], &mut cancelled)?
                    .is_eq()
            {
                values.swap(write, read);
                write += 1;
            }
        }
        values.truncate(write);
        if cancelled() {
            return Err("IDB scope cancelled");
        }
        // All original string leases stay charged conservatively until scope
        // drops, including removed duplicates; no undercharge after dedup.
        Ok(self)
    }
    /// Transfer names into the existing public command/record shape. The
    /// returned custody MUST be stored alongside that exact Vec and dropped
    /// only after the Vec is dropped. No cloning or allocation occurs here.
    pub fn into_parts(self) -> (Vec<String>, ScopeCustody) {
        let Scope { names, strings } = self;
        let Buffer { values, lease } = names;
        (
            values,
            ScopeCustody {
                _vector: lease,
                _strings: strings,
                metadata: None,
            },
        )
    }
    pub fn allocate(
        ledger: &Arc<Ledger>,
        count: usize,
        limit: usize,
        used: usize,
    ) -> Result<Self, &'static str> {
        let names = Buffer::allocate(ledger, count, limit, used)?;
        let strings = Buffer::allocate(ledger, count, limit, used)?;
        Ok(Self { names, strings })
    }
    pub fn push(&mut self, text: Text) -> Result<(), &'static str> {
        if self.names.values.len() == self.names.values.capacity()
            || self.strings.values.len() == self.strings.values.capacity()
        {
            return Err("IDB scope capacity");
        }
        let Text { value, lease } = text;
        self.names.push(value)?;
        self.strings.push(lease)
    }
    pub fn names(&self) -> &[String] {
        self.names.as_slice()
    }
    pub fn clone_admitted(
        &self,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
    ) -> Result<Self, &'static str> {
        self.clone_admitted_cancelled(ledger, limit, used, || false)
    }
    pub fn clone_admitted_cancelled(
        &self,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("IDB scope cancelled");
        }
        let mut copy = Self::allocate(ledger, self.names().len(), limit, used)?;
        for name in self.names() {
            copy.push(Text::copy_string_cancelled(
                ledger,
                name,
                limit,
                used,
                &mut cancelled,
            )?)?;
        }
        if cancelled() {
            return Err("IDB scope cancelled");
        }
        Ok(copy)
    }
}

fn compare_names_cancelled(
    a: &str,
    b: &str,
    cancelled: &mut dyn FnMut() -> bool,
) -> Result<std::cmp::Ordering, &'static str> {
    // Raw marker strings only occur for reserved prefix/lone-surrogate values.
    // Actual UTF16 comparison is used; allocation-free decoder below.
    fn encoded(s: &str) -> Option<&str> {
        s.strip_prefix(oryn_dom::dom_string::PREFIX)?
            .strip_prefix('[')?
            .strip_suffix(']')
    }
    let a_encoded = encoded(a);
    let b_encoded = encoded(b);
    let mut au = a.encode_utf16();
    let mut bu = b.encode_utf16();
    let mut ae = a_encoded.unwrap_or("").split(',');
    let mut be = b_encoded.unwrap_or("").split(',');
    let mut offset = 0usize;
    loop {
        if offset.is_multiple_of(1024) && cancelled() {
            return Err("IDB scope cancelled");
        }
        offset += 1;
        let av = if a_encoded.is_some() {
            ae.next().and_then(|n| n.parse::<u16>().ok())
        } else {
            au.next()
        };
        let bv = if b_encoded.is_some() {
            be.next().and_then(|n| n.parse::<u16>().ok())
        } else {
            bu.next()
        };
        match av.cmp(&bv) {
            std::cmp::Ordering::Equal if av.is_some() => {}
            order => return Ok(order),
        }
    }
}

pub enum Admission {
    Begin(BeginAdmission),
    CreateIndex(CreateIndexAdmission),
    Metadata(metadata_command::OwnedMetadataCommand),
}
pub struct BeginAdmission {
    token: String,
    database: String,
    mode: String,
    connection: Option<String>,
    scope: Scope,
    // Physical database String above must be released before its original lease.
    _database_custody: Option<Lease>,
}
impl Admission {
    pub fn is_begin(&self) -> bool {
        matches!(self, Self::Begin(_))
    }
    pub fn new(
        token: String,
        database: String,
        scope: Scope,
        mode: String,
        connection: Option<String>,
    ) -> Self {
        Self::Begin(BeginAdmission {
            token,
            database,
            scope,
            mode,
            connection,
            _database_custody: None,
        })
    }
    pub fn new_with_database_custody(
        token: String,
        database: String,
        scope: Scope,
        mode: String,
        connection: Option<String>,
        database_custody: Lease,
    ) -> Self {
        Self::Begin(BeginAdmission {
            token,
            database,
            scope,
            mode,
            connection,
            _database_custody: Some(database_custody),
        })
    }
    pub fn prepare(
        &self,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
    ) -> Result<PreparedAdmission, &'static str> {
        self.prepare_cancelled(ledger, limit, used, || false)
    }
    pub fn prepare_cancelled(
        &self,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedAdmission, &'static str> {
        if cancelled() {
            return Err("IDB admission cancelled");
        }
        if let Self::Metadata(command) = self {
            return metadata_command::OwnedMetadataCommand::copy(
                command.command(),
                ledger,
                limit,
                used,
                &mut cancelled,
            )
            .map(PreparedAdmission::Metadata);
        }
        let Self::Begin(this) = self else {
            let Self::CreateIndex(index) = self else {
                unreachable!()
            };
            return index
                .prepare_cancelled(ledger, limit, used, &mut cancelled)
                .map(PreparedAdmission::CreateIndex);
        };
        let token = Text::copy_string_cancelled(ledger, &this.token, limit, used, &mut cancelled)?;
        let database =
            Text::copy_string_cancelled(ledger, &this.database, limit, used, &mut cancelled)?;
        let mode = Text::copy_string_cancelled(ledger, &this.mode, limit, used, &mut cancelled)?;
        let connection = this
            .connection
            .as_deref()
            .map(|value| Text::copy_string_cancelled(ledger, value, limit, used, &mut cancelled))
            .transpose()?;
        let scope = this
            .scope
            .clone_admitted_cancelled(ledger, limit, used, &mut cancelled)?;
        if cancelled() {
            return Err("IDB admission cancelled");
        }
        let Text {
            value: token,
            lease: token_lease,
        } = token;
        let Text {
            value: database,
            lease: database_lease,
        } = database;
        let Text {
            value: mode,
            lease: mode_lease,
        } = mode;
        let (connection, connection_lease) = match connection {
            Some(Text { value, lease }) => (Some(value), Some(lease)),
            None => (None, None),
        };
        Ok(PreparedAdmission::Begin(PreparedBeginAdmission {
            token,
            database,
            mode,
            connection,
            scope,
            metadata_custody: Some((token_lease, database_lease, mode_lease, connection_lease)),
        }))
    }
}
pub enum PreparedAdmission {
    Begin(PreparedBeginAdmission),
    CreateIndex(PreparedCreateIndex),
    Metadata(metadata_command::OwnedMetadataCommand),
}
pub struct PreparedBeginAdmission {
    token: String,
    database: String,
    mode: String,
    connection: Option<String>,
    scope: Scope,
    metadata_custody: Option<(Lease, Lease, Lease, Option<Lease>)>,
}
impl PreparedAdmission {
    pub fn into_command(self) -> (crate::platform::IdbHostCommand, CommandCustody) {
        if let Self::Metadata(command) = self {
            let (command, custody) = command.into_parts();
            return (command, CommandCustody::Metadata(custody));
        }
        let Self::Begin(this) = self else {
            let Self::CreateIndex(index) = self else {
                unreachable!()
            };
            return index.into_command();
        };
        let (scope, mut custody) = this.scope.into_parts();
        custody.metadata = this.metadata_custody;
        (
            crate::platform::IdbHostCommand::BeginTransaction {
                token: this.token,
                database: this.database,
                scope,
                mode: this.mode,
                connection: this.connection,
            },
            CommandCustody::Begin(custody),
        )
    }
}

pub enum CommandCustody {
    Begin(ScopeCustody),
    CreateIndex(IndexCustody),
    Metadata(Buffer<Lease>),
}
pub struct CreateIndexAdmission {
    database: Text,
    store: Text,
    name: Text,
    key_path: Text,
    unique: bool,
    multi_entry: bool,
    upgrade: Option<(Text, Text)>,
}
pub struct PreparedCreateIndex {
    index: CreateIndexAdmission,
    inner: Option<Buffer<crate::platform::IdbHostCommand>>,
}
pub struct IndexCustody {
    _database: Lease,
    _store: Lease,
    _name: Lease,
    _key_path: Lease,
    _upgrade: Option<(Lease, Lease, Lease)>,
}
impl Admission {
    pub fn metadata(
        command: &crate::platform::IdbHostCommand,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        metadata_command::OwnedMetadataCommand::copy(command, ledger, limit, used, cancelled)
            .map(Self::Metadata)
    }
    pub fn create_index(
        database: Text,
        store: Text,
        name: Text,
        key_path: Text,
        unique: bool,
        multi_entry: bool,
        upgrade: Option<(Text, Text)>,
    ) -> Self {
        Self::CreateIndex(CreateIndexAdmission {
            database,
            store,
            name,
            key_path,
            unique,
            multi_entry,
            upgrade,
        })
    }
}
impl CreateIndexAdmission {
    fn prepare_cancelled(
        &self,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedCreateIndex, &'static str> {
        if cancelled() {
            return Err("IDB index cancelled");
        }
        let mut copy = |text: &Text| {
            Text::copy_string_cancelled(ledger, text.as_str(), limit, used, &mut cancelled)
        };
        let index = Self {
            database: copy(&self.database)?,
            store: copy(&self.store)?,
            name: copy(&self.name)?,
            key_path: copy(&self.key_path)?,
            unique: self.unique,
            multi_entry: self.multi_entry,
            upgrade: self
                .upgrade
                .as_ref()
                .map(|(token, name)| Ok::<_, &'static str>((copy(token)?, copy(name)?)))
                .transpose()?,
        };
        let inner = if index.upgrade.is_some() {
            Some(Buffer::allocate(ledger, 1, limit, used)?)
        } else {
            None
        };
        if cancelled() {
            return Err("IDB index cancelled");
        }
        Ok(PreparedCreateIndex { index, inner })
    }
}
impl PreparedCreateIndex {
    fn into_command(self) -> (crate::platform::IdbHostCommand, CommandCustody) {
        use crate::platform::{IdbHostCommand, IdbIndexDefinition};
        let CreateIndexAdmission {
            database,
            store,
            name,
            key_path,
            unique,
            multi_entry,
            upgrade,
        } = self.index;
        let Text {
            value: database,
            lease: database_lease,
        } = database;
        let Text {
            value: store,
            lease: store_lease,
        } = store;
        let Text {
            value: name,
            lease: name_lease,
        } = name;
        let Text {
            value: key_path,
            lease: key_path_lease,
        } = key_path;
        let command = IdbHostCommand::CreateIndex {
            database,
            store,
            definition: IdbIndexDefinition {
                name,
                key_path,
                unique,
                multi_entry,
            },
        };
        let (command, upgrade_custody) = if let Some((token, name)) = upgrade {
            let Text {
                value: token,
                lease: token_lease,
            } = token;
            let Text {
                value: name,
                lease: upgrade_name_lease,
            } = name;
            let mut inner = self.inner.expect("prepared upgrade allocation");
            inner
                .push(command)
                .unwrap_or_else(|_| unreachable!("exact single command allocation"));
            let Buffer { values, lease } = inner;
            // Exact capacity/length one was allocated using Layout::array<T>(1),
            // identical to Layout::new<T>(); consume it without reallocation.
            let mut values = std::mem::ManuallyDrop::new(values);
            let inner = unsafe { Box::from_raw(values.as_mut_ptr()) };
            (
                IdbHostCommand::UpgradeConnection { token, name, inner },
                Some((token_lease, upgrade_name_lease, lease)),
            )
        } else {
            (command, None)
        };
        (
            command,
            CommandCustody::CreateIndex(IndexCustody {
                _database: database_lease,
                _store: store_lease,
                _name: name_lease,
                _key_path: key_path_lease,
                _upgrade: upgrade_custody,
            }),
        )
    }
}

#[cfg(test)]
mod custody_tests {
    use super::*;
    #[test]
    fn retained_heap_receipt_survives_external_settlement_and_shared_idb_reserves() {
        let ledger = Arc::new(Ledger::default());
        let receipt = ledger.heap_receipt(100, 10).unwrap();
        let other = Arc::new(Ledger::default());
        assert!(other.reserve_from_receipt(1, &receipt).is_err());
        assert_eq!(other.live_bytes(), 0);
        let retained = ledger.reserve_from_receipt(50, &receipt).unwrap();
        let adjustment = ledger.begin_adjustment().unwrap().unwrap();
        ledger.finish_adjustment(adjustment).unwrap();
        // Custom V8 external accounting does not enter HeapStatistics backing
        // stores: fresh snapshots remain 10 after settling 50 native bytes.
        let fresh = ledger.heap_receipt(100, 10).unwrap();
        assert!(ledger.reserve_from_receipt(41, &fresh).is_err());
        let idb = ledger.reserve(20, 100, 10).unwrap();
        let text = Text::copy_string_from_receipt(&ledger, "1234567890", &receipt, || false).unwrap();
        assert_eq!(ledger.live_bytes(), 80);
        assert!(ledger.reserve_from_receipt(11, &receipt).is_err());
        drop(idb); drop(text); drop(retained);
        assert_eq!(ledger.live_bytes(), 0);
        let adjustment = ledger.begin_adjustment().unwrap().unwrap();
        ledger.finish_adjustment(adjustment).unwrap();
        assert!(ledger.reserve(90, 100, 10).is_ok());
    }
    #[test]
    fn retained_receipt_text_cancellation_releases_entry_chunk_and_final_allocations() {
        let ledger = Arc::new(Ledger::default());
        let receipt = ledger.heap_receipt(32768, 0).unwrap();
        for (value, cancel_at) in [("", 1), ("x", 3), ("x", 1)] {
            let mut polls = 0;
            assert!(Text::copy_string_from_receipt(&ledger, value, &receipt, || {
                polls += 1;
                polls == cancel_at
            }).is_err());
            assert_eq!(ledger.live_bytes(), 0);
        }
        let long = "x".repeat(8192);
        let mut polls = 0;
        assert!(Text::copy_string_from_receipt(&ledger, &long, &receipt, || {
            polls += 1;
            polls == 4
        }).is_err());
        assert_eq!(ledger.live_bytes(), 0);
    }
    #[test]
    fn scope_copy_cancellation_entry_fill_and_final_release() {
        let ledger = Arc::new(Ledger::default());
        for names in [vec![], vec!["x".into()], vec!["x".repeat(8192)]] {
            assert!(Scope::copy_names_cancelled(&ledger, &names, 1_000_000, 0, || true).is_err());
            assert_eq!(ledger.live_bytes(), 0);
            let mut total = 0;
            drop(
                Scope::copy_names_cancelled(&ledger, &names, 1_000_000, 0, || {
                    total += 1;
                    false
                })
                .unwrap(),
            );
            let mut calls = 0;
            assert!(
                Scope::copy_names_cancelled(&ledger, &names, 1_000_000, 0, || {
                    calls += 1;
                    calls == total
                })
                .is_err()
            );
            assert_eq!(calls, total);
            assert_eq!(ledger.live_bytes(), 0);
        }
    }
    #[test]
    fn cancelled_normalization_preserves_utf16_order_and_releases() {
        let ledger = Arc::new(Ledger::default());
        let names = [
            "z".into(),
            "a".into(),
            "a".into(),
            "😀".into(),
            "\u{e000}".into(),
        ];
        let sorted = Scope::copy_names(&ledger, &names, 1_000_000, 0)
            .unwrap()
            .normalize_cancelled(|| false)
            .unwrap();
        assert_eq!(sorted.names(), &["a", "z", "😀", "\u{e000}"]);
        drop(sorted);
        assert_eq!(ledger.live_bytes(), 0);
        let scope = Scope::copy_names(&ledger, &[], 1_000_000, 0).unwrap();
        assert!(scope.normalize_cancelled(|| true).is_err());
        assert_eq!(ledger.live_bytes(), 0);
        let scope = Scope::copy_names(&ledger, &names, 1_000_000, 0).unwrap();
        let mut calls = 0;
        assert!(
            scope
                .normalize_cancelled(|| {
                    calls += 1;
                    calls > 4
                })
                .is_err()
        );
        assert_eq!(ledger.live_bytes(), 0);
    }
    #[test]
    fn cancelled_prepare_empty_and_nonempty_retains_only_original() {
        let ledger = Arc::new(Ledger::default());
        for value in ["", "x"] {
            let text = |v| Text::copy_string(&ledger, v, 1_000_000, 0).unwrap();
            let index = Admission::create_index(
                text(value),
                text(value),
                text(value),
                text(value),
                false,
                false,
                None,
            );
            let begin = Admission::new(
                value.into(),
                value.into(),
                Scope::copy_names(&ledger, &[], 1_000_000, 0).unwrap(),
                value.into(),
                None,
            );
            for admission in [&index, &begin] {
                let baseline = ledger.live_bytes();
                assert!(
                    admission
                        .prepare_cancelled(&ledger, 1_000_000, 0, || true)
                        .is_err()
                );
                assert_eq!(ledger.live_bytes(), baseline);
                let mut count = 0;
                admission
                    .prepare_cancelled(&ledger, 1_000_000, 0, || {
                        count += 1;
                        false
                    })
                    .unwrap();
                assert_eq!(ledger.live_bytes(), baseline);
                let mut calls = 0;
                assert!(
                    admission
                        .prepare_cancelled(&ledger, 1_000_000, 0, || {
                            calls += 1;
                            calls == count
                        })
                        .is_err()
                );
                assert_eq!(calls, count);
                assert_eq!(ledger.live_bytes(), baseline);
            }
            drop(begin);
            drop(index);
            assert_eq!(ledger.live_bytes(), 0);
        }
    }
    #[test]
    fn begin_retains_original_database_copy_through_attempts() {
        let ledger = Arc::new(Ledger::default());
        let database = Text::copy_string(&ledger, "database", 1_000_000, 0).unwrap();
        let Text {
            value: database,
            lease,
        } = database;
        let original = ledger.live_bytes();
        let scope = Scope::copy_names(&ledger, &[], 1_000_000, 0).unwrap();
        let admission = Admission::new_with_database_custody(
            "token".into(),
            database,
            scope,
            "readonly".into(),
            None,
            lease,
        );
        assert_eq!(ledger.live_bytes(), original);
        let attempt = admission.prepare(&ledger, 1_000_000, 0).unwrap();
        assert!(ledger.live_bytes() > original);
        drop(attempt);
        assert_eq!(ledger.live_bytes(), original);
        drop(admission);
        assert_eq!(ledger.live_bytes(), 0);
    }
    #[test]
    fn create_index_attempt_copies_are_independent_and_release_after_dto() {
        let ledger = Arc::new(Ledger::default());
        let text = |value: &str| Text::copy_string(&ledger, value, 1_000_000, 0).unwrap();
        let admission = Admission::create_index(
            text("db"),
            text("store"),
            text("index"),
            text("path"),
            true,
            false,
            Some((text("token"), text("db"))),
        );
        let retained = ledger.live_bytes();
        let first = admission.prepare(&ledger, 1_000_000, 0).unwrap();
        let with_first = ledger.live_bytes();
        assert!(with_first > retained);
        let second = admission.prepare(&ledger, 1_000_000, 0).unwrap();
        assert_eq!(ledger.live_bytes(), retained + 2 * (with_first - retained));
        let (command, custody) = first.into_command();
        let crate::platform::IdbHostCommand::UpgradeConnection { inner, .. } = &command else {
            panic!("missing upgrade provenance")
        };
        assert!(
            matches!(&**inner,crate::platform::IdbHostCommand::CreateIndex { definition,.. }
            if definition.name == "index" && definition.key_path == "path" && definition.unique)
        );
        drop(command);
        assert_eq!(ledger.live_bytes(), retained + 2 * (with_first - retained));
        drop(custody);
        assert_eq!(ledger.live_bytes(), with_first);
        drop(second);
        assert_eq!(ledger.live_bytes(), retained);
        assert!(admission.prepare(&ledger, retained, 0).is_err());
        assert_eq!(ledger.live_bytes(), retained);
        drop(admission);
        assert_eq!(ledger.live_bytes(), 0);
    }
    #[test]
    fn discarded_adjustment_restores_pending_release() {
        let ledger = Arc::new(Ledger::default());
        let lease = ledger.reserve(16, 100, 0).unwrap();
        let adjustment = ledger.begin_adjustment().unwrap().unwrap();
        drop(lease);
        drop(adjustment);
        assert_eq!(ledger.live_bytes(), 0);
        assert!(ledger.begin_adjustment().unwrap().is_none());
    }
    #[test]
    fn callback_release_during_adjustment_needs_next_receipt() {
        let ledger = Arc::new(Ledger::default());
        let lease = ledger.reserve(16, 100, 0).unwrap();
        let first = ledger.begin_adjustment().unwrap().unwrap();
        drop(lease);
        assert!(ledger.begin_adjustment().unwrap().is_none());
        ledger.finish_adjustment(first).unwrap();
        let next = ledger.begin_adjustment().unwrap().unwrap();
        assert_eq!(next.amount(), -16);
        ledger.finish_adjustment(next).unwrap();
        assert_eq!(ledger.live_bytes(), 0);
    }
    #[test]
    fn native_scope_clone_release_and_encoding_match() {
        let ledger = Arc::new(Ledger::default());
        let names = vec!["one".into(), oryn_dom::dom_string::from_units(&[0xd800])];
        let scope = Scope::copy_names(&ledger, &names, 10000, 0).unwrap();
        let charged = ledger.live_bytes();
        let copy = scope.clone_admitted(&ledger, 10000, 0).unwrap();
        assert_eq!(ledger.live_bytes(), 2 * charged);
        assert_eq!(scope.names(), copy.names());
        drop(copy);
        assert_eq!(ledger.live_bytes(), charged);
        drop(scope);
        assert_eq!(ledger.live_bytes(), 0);
    }
}

#[cfg(test)]
mod text_units_cancellation_boundary_tests {
    use super::*;
    #[test]
    fn empty_entry_and_preallocation_cancellation_precede_ledger_reservation() {
        let ledger = Arc::new(Ledger::default());
        for cancel_at in [1, 2] {
            let mut polls = 0;
            assert!(Text::from_units_cancelled(&ledger, &[], 1024, 0, || {
                polls += 1;
                assert_eq!(ledger.live_bytes(), 0);
                polls == cancel_at
            }).is_err());
            assert_eq!(polls, cancel_at);
            assert_eq!(ledger.live_bytes(), 0);
        }
    }
    #[test]
    fn final_and_invalid_surrogate_cancellation_release_initialized_payload() {
        let ledger = Arc::new(Ledger::default());
        // Entry, decode, preallocation, encode, final: final observes the
        // initialized payload before rejecting and releasing its custody.
        for units in [&[b'x' as u16][..], &[0xd800][..]] {
            let mut polls = 0;
            assert!(Text::from_units_cancelled(&ledger, units, 1024, 0, || {
                polls += 1;
                if polls == 5 { assert!(ledger.live_bytes() > 0); }
                polls == 5
            }).is_err());
            assert_eq!(polls, 5);
            assert_eq!(ledger.live_bytes(), 0);
            let text = Text::from_units_cancelled(&ledger, units, 1024, 0, || false).unwrap();
            assert_eq!(text.as_str(), oryn_dom::dom_string::from_units(units));
            drop(text);
            assert_eq!(ledger.live_bytes(), 0);
        }
    }
}
