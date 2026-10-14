//! Internal thread-safe ownership and accounting for IndexedDB admission payloads.
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Ledger(Mutex<State>);
#[derive(Default)]
struct State {
    live: usize,
    accounted: usize,
    delta: i64,
    flushing: bool,
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
        let other = used
            .checked_sub(state.accounted)
            .ok_or("IDB allocation accounting")?;
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
    pub fn copy_string(
        ledger: &Arc<Ledger>,
        value: &str,
        limit: usize,
        used: usize,
    ) -> Result<Self, &'static str> {
        let mut bytes = Buffer::<u8>::allocate(ledger, value.len(), limit, used)?;
        for byte in value.bytes() {
            bytes.push(byte)?
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
        let mut valid = true;
        let mut unicode_bytes = 0usize;
        for scalar in char::decode_utf16(units.iter().copied()) {
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
        let mut buffer = Buffer::<u8>::allocate(ledger, capacity, limit, used)?;
        if encoded {
            for byte in prefix.bytes().chain(std::iter::once(b'[')) {
                buffer.push(byte)?
            }
            for (index, unit) in units.iter().copied().enumerate() {
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
            for scalar in char::decode_utf16(units.iter().copied()) {
                let scalar = scalar.map_err(|_| "IDB UTF16 changed")?;
                let mut bytes = [0u8; 4];
                for byte in scalar.encode_utf8(&mut bytes).bytes() {
                    buffer.push(byte)?
                }
            }
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
        let mut copy = Self::allocate(ledger, names.len(), limit, used)?;
        for name in names {
            let mut bytes = Buffer::<u8>::allocate(ledger, name.len(), limit, used)?;
            for byte in name.bytes() {
                bytes.push(byte)?
            }
            let Buffer { values, lease } = bytes;
            copy.push(Text {
                value: String::from_utf8(values).map_err(|_| "IDB UTF8")?,
                lease,
            })?;
        }
        Ok(copy)
    }
    pub fn normalize(mut self) -> Result<Self, &'static str> {
        // Sorting borrows stored strings and never allocates per-comparison
        // UTF16 key vectors. Decimal reserved representations decode lazily.
        self.names
            .values
            .sort_unstable_by(|a, b| compare_names(a, b));
        self.names.values.dedup();
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
        let mut copy = Self::allocate(ledger, self.names().len(), limit, used)?;
        for name in self.names() {
            let mut bytes = Buffer::<u8>::allocate(ledger, name.len(), limit, used)?;
            for byte in name.bytes() {
                bytes.push(byte)?
            }
            let Buffer { values, lease } = bytes;
            let value = String::from_utf8(values).map_err(|_| "IDB scope UTF8")?;
            copy.push(Text { value, lease })?;
        }
        Ok(copy)
    }
}

fn compare_names(a: &str, b: &str) -> std::cmp::Ordering {
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
    loop {
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
            order => return order,
        }
    }
}

pub struct Admission {
    token: String,
    database: String,
    mode: String,
    connection: Option<String>,
    scope: Scope,
}
impl Admission {
    pub fn new(
        token: String,
        database: String,
        scope: Scope,
        mode: String,
        connection: Option<String>,
    ) -> Self {
        Self {
            token,
            database,
            scope,
            mode,
            connection,
        }
    }
    pub fn prepare(
        &self,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
    ) -> Result<PreparedAdmission, &'static str> {
        let token = Text::copy_string(ledger, &self.token, limit, used)?;
        let database = Text::copy_string(ledger, &self.database, limit, used)?;
        let mode = Text::copy_string(ledger, &self.mode, limit, used)?;
        let connection = self
            .connection
            .as_deref()
            .map(|value| Text::copy_string(ledger, value, limit, used))
            .transpose()?;
        let scope = self.scope.clone_admitted(ledger, limit, used)?;
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
        Ok(PreparedAdmission {
            token,
            database,
            mode,
            connection,
            scope,
            metadata_custody: Some((token_lease, database_lease, mode_lease, connection_lease)),
        })
    }
}
pub struct PreparedAdmission {
    token: String,
    database: String,
    mode: String,
    connection: Option<String>,
    scope: Scope,
    metadata_custody: Option<(Lease, Lease, Lease, Option<Lease>)>,
}
impl PreparedAdmission {
    pub fn into_command(self) -> (crate::platform::IdbHostCommand, ScopeCustody) {
        let (scope, mut custody) = self.scope.into_parts();
        custody.metadata = self.metadata_custody;
        (
            crate::platform::IdbHostCommand::BeginTransaction {
                token: self.token,
                database: self.database,
                scope,
                mode: self.mode,
                connection: self.connection,
            },
            custody,
        )
    }
}

#[cfg(test)]
mod custody_tests {
    use super::*;
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
