//! Staged exact-custody name snapshots for current and rollback database lists.
use crate::idb_owned_admission::{Buffer, Ledger, Scope, ScopeCustody, Text};
use std::sync::Arc;

pub(crate) struct Names {
    values: Vec<String>,
    _custody: ScopeCustody,
}
impl Names {
    pub(crate) fn copy_text(
        value: &str,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Text, &'static str> {
        if cancelled() {
            return Err("names cancelled");
        }
        let mut bytes = Buffer::<u8>::allocate(ledger, value.len(), limit, used)?;
        for (offset, byte) in value.bytes().enumerate() {
            if offset % 1024 == 0 && cancelled() {
                return Err("names cancelled");
            }
            bytes.push(byte)?;
        }
        if cancelled() {
            return Err("names cancelled");
        }
        Text::from_admitted_utf8(bytes)
    }
    pub(crate) fn from_strings(
        values: &[String],
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("names cancelled");
        }
        let mut scope = Scope::allocate(ledger, values.len(), limit, used)?;
        for value in values {
            scope.push(Text::copy_string_cancelled(
                ledger,
                value,
                limit,
                used,
                &mut *cancelled,
            )?)?;
        }
        if cancelled() {
            return Err("names cancelled");
        }
        let (values, custody) = scope.into_parts();
        Ok(Self { values, _custody: custody })
    }
    pub(crate) fn as_slice(&self) -> &[String] {
        &self.values
    }
    pub(crate) fn copy_admitted(
        &self,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        Self::from_strings(&self.values, ledger, limit, used, cancelled)
    }
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.values.iter().any(|value| value == name)
    }
    pub(crate) fn inserted(
        &self,
        name: &str,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("names cancelled");
        }
        let len = self.values.len().checked_add(1).ok_or("names overflow")?;
        let mut scope = Scope::allocate(ledger, len, limit, used)?;
        for value in &self.values {
            scope.push(Self::copy_text(value, ledger, limit, used, &mut cancelled)?)?;
        }
        scope.push(Self::copy_text(name, ledger, limit, used, &mut cancelled)?)?;
        if cancelled() {
            return Err("names cancelled");
        }
        let (values, custody) = scope.into_parts();
        Ok(Self { values, _custody: custody })
    }
    pub(crate) fn removed(
        &self,
        name: &str,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("names cancelled");
        }
        let mut count = 0usize;
        for value in &self.values {
            if cancelled() {
                return Err("names cancelled");
            }
            if value.as_str() != name {
                count += 1
            }
        }
        let mut scope = Scope::allocate(ledger, count, limit, used)?;
        for value in &self.values {
            if cancelled() {
                return Err("names cancelled");
            }
            if value.as_str() != name {
                scope.push(Self::copy_text(value, ledger, limit, used, &mut cancelled)?)?;
            }
        }
        if cancelled() {
            return Err("names cancelled");
        }
        let (values, custody) = scope.into_parts();
        Ok(Self { values, _custody: custody })
    }
    pub(crate) fn renamed(
        &self,
        old: &str,
        new: &str,
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("names cancelled");
        }
        let mut scope = Scope::allocate(ledger, self.values.len(), limit, used)?;
        for value in &self.values {
            scope.push(Self::copy_text(
                if value == old { new } else { value },
                ledger,
                limit,
                used,
                &mut cancelled,
            )?)?;
        }
        if cancelled() {
            return Err("names cancelled");
        }
        let (values, custody) = scope.into_parts();
        Ok(Self { values, _custody: custody })
    }
    pub(crate) fn from_metadata(
        values: &[super::owned_index_metadata::StoreMetadata],
        ledger: &Arc<Ledger>,
        limit: usize,
        used: usize,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<Self, &'static str> {
        if cancelled() {
            return Err("names cancelled");
        }
        let mut scope = Scope::allocate(ledger, values.len(), limit, used)?;
        for value in values {
            scope.push(crate::idb_owned_admission::Text::copy_string_cancelled(
                ledger,
                value.name.as_str(),
                limit,
                used,
                &mut *cancelled,
            )?)?;
        }
        if cancelled() {
            return Err("names cancelled");
        }
        let (values, custody) = scope.into_parts();
        Ok(Self { values, _custody: custody })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_long_and_many_name_mutations_release_attempts() {
        let ledger = Arc::new(Ledger::default());
        let current =
            Names::from_strings(&["existing".into()], &ledger, 1 << 20, 0, &mut || false).unwrap();
        let baseline = ledger.live_bytes();
        let long = "x".repeat(8192);
        let mut calls = 0;
        assert!(
            current
                .inserted(&long, &ledger, 1 << 20, 0, || {
                    calls += 1;
                    calls > 7
                })
                .is_err()
        );
        assert_eq!(ledger.live_bytes(), baseline);
        let source = vec!["same".to_owned(); 2048];
        let many = Names::from_strings(&source, &ledger, 1 << 20, 0, &mut || false).unwrap();
        let retained = ledger.live_bytes();
        let mut calls = 0;
        assert!(
            many.renamed("same", "new", &ledger, 1 << 20, 0, || {
                calls += 1;
                calls > 100
            })
            .is_err()
        );
        assert_eq!(ledger.live_bytes(), retained);
        let mut calls = 0;
        assert!(
            many.removed("missing", &ledger, 1 << 20, 0, || {
                calls += 1;
                calls > 100
            })
            .is_err()
        );
        assert_eq!(ledger.live_bytes(), retained);
        drop(many);
        drop(current);
        assert_eq!(ledger.live_bytes(), 0);
    }
    #[test]
    fn rollback_names_copy_charge_survives_current_snapshot_drop() {
        let ledger = Arc::new(Ledger::default());
        let current =
            Names::from_strings(&["store".into()], &ledger, 4096, 0, &mut || false).unwrap();
        let initial = ledger.live_bytes();
        let rollback = current
            .copy_admitted(&ledger, 4096, 0, &mut || false)
            .unwrap();
        assert_eq!(ledger.live_bytes(), initial * 2);
        drop(current);
        assert_eq!(ledger.live_bytes(), initial);
        assert!(rollback.contains("store"));
        drop(rollback);
        assert_eq!(ledger.live_bytes(), 0);
    }
}
