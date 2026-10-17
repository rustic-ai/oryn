// Prepared character geometry: one payload allocation/lease, no heap lease list.
use super::*;
type BoundsLease = Option<std::sync::Arc<dyn Send + Sync>>;
#[derive(Default)]
pub(super) struct PreparedBounds {
    values: Vec<[f64; 4]>,
    lease: BoundsLease,
}
impl PreparedBounds {
    pub(super) fn values(&self) -> &[[f64; 4]] { &self.values }
    pub(super) fn push(
        &mut self, value: [f64; 4],
        mut reserve: impl FnMut(usize) -> Result<BoundsLease, V8BindingError>,
    ) -> Result<(), V8BindingError> {
        if self.values.len() == self.values.capacity() {
            let capacity = self.values.capacity().max(4).checked_mul(2)
                .filter(|capacity| *capacity <= i32::MAX as usize)
                .ok_or(V8BindingError::Allocation("EditContext geometry capacity"))?;
            let bytes = capacity.checked_mul(std::mem::size_of::<[f64; 4]>())
                .ok_or(V8BindingError::Allocation("EditContext geometry bytes"))?;
            // Old payload and its lease remain live during complete new-buffer
            // admission and allocation. No in-place realloc hides their peak.
            let lease = reserve(bytes)?;
            let mut next = Vec::new();
            next.try_reserve_exact(capacity)
                .map_err(|_| V8BindingError::Allocation("EditContext geometry allocation"))?;
            if next.capacity() != capacity {
                return Err(V8BindingError::Allocation("EditContext geometry allocation capacity"));
            }
            next.extend_from_slice(&self.values);
            next.push(value);
            self.values = next;
            self.lease = lease;
        } else {
            self.values.push(value);
        }
        Ok(())
    }
}

#[cfg(test)]
mod geometry_controls {
    use super::*;
    #[test]
    fn failed_growth_keeps_original_geometry_and_lease_until_drop() {
        let owner=std::sync::Arc::new(());
        let mut geometry=PreparedBounds::default();
        for _ in 0..8 {
            geometry.push([1.,2.,3.,4.], |_| Ok(Some(owner.clone()))).unwrap();
        }
        assert_eq!(std::sync::Arc::strong_count(&owner),2);
        assert!(geometry.push([5.;4], |_| Err(V8BindingError::Allocation("denied"))).is_err());
        assert_eq!(geometry.values(),&[[1.,2.,3.,4.];8]);
        assert_eq!(std::sync::Arc::strong_count(&owner),2);
        drop(geometry);
        assert_eq!(std::sync::Arc::strong_count(&owner),1);
    }
    #[test]
    fn growth_admission_observes_old_payload_and_exact_new_capacity() {
        let owner=std::sync::Arc::new(());
        let mut geometry=PreparedBounds::default();
        for _ in 0..8 {geometry.push([0.;4], |_|Ok(Some(owner.clone()))).unwrap();}
        geometry.push([1.;4], |bytes| {
            assert_eq!(bytes,16*std::mem::size_of::<[f64;4]>());
            assert_eq!(std::sync::Arc::strong_count(&owner),2);
            Ok(Some(owner.clone()))
        }).unwrap();
        assert_eq!(geometry.values().len(),9);
        assert_eq!(std::sync::Arc::strong_count(&owner),2);
    }
}
