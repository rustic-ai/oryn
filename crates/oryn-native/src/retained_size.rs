//! Conservative retained allocation accounting. Container capacities and owned
//! buffers are charged; allocator implementation bookkeeping is not measured.
#![cfg(feature = "v8-host")]

pub(crate) struct Size(pub(crate) usize);
impl Size {
    pub(crate) fn add(&mut self, n: usize) -> Result<(), String> {
        self.0 = self
            .0
            .checked_add(n)
            .ok_or_else(|| "retained allocation accounting overflow".to_owned())?;
        Ok(())
    }
    pub(crate) fn string(&mut self, s: &String) -> Result<(), String> {
        self.add(s.capacity())
    }
    pub(crate) fn optional(&mut self, s: &Option<String>) -> Result<(), String> {
        if let Some(s) = s {
            self.string(s)?
        }
        Ok(())
    }
    pub(crate) fn vector<T>(&mut self, v: &Vec<T>) -> Result<(), String> {
        self.add(
            v.capacity()
                .checked_mul(std::mem::size_of::<T>())
                .ok_or("retained allocation accounting overflow")?,
        )
    }
    pub(crate) fn headers(&mut self, v: &Vec<(String, String)>) -> Result<(), String> {
        self.vector(v)?;
        for (k, v) in v {
            self.string(k)?;
            self.string(v)?;
        }
        Ok(())
    }
    // Charge one full generously sized tree node per entry. This deliberately
    // overestimates sparse/internal BTree nodes rather than claiming precision.
    pub(crate) fn tree<K, V>(
        &mut self,
        v: &std::collections::BTreeMap<K, V>,
    ) -> Result<(), String> {
        let node = std::mem::size_of::<(K, V)>()
            .checked_mul(16)
            .and_then(|n| n.checked_add(256))
            .ok_or("retained tree accounting overflow")?;
        self.add(
            v.len()
                .checked_mul(node)
                .ok_or("retained tree accounting overflow")?,
        )
    }
}
