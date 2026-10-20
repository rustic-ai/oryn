/// Explicit caller-owned allocation admission for projection scratch/output.
/// Existing DOM interfaces use OrdinaryProjectionAllocation; native callers
/// can retain charges with their owning graph/cache without a global context.
pub trait ProjectionAllocation {
    type Error: From<crate::DomError>;
    fn reserve(&mut self, bytes: usize) -> Result<(), Self::Error>;
    fn allocation_error(&self, error: std::collections::TryReserveError) -> Self::Error;
    fn scratch_mark(&self) -> usize {
        0
    }
    fn release_scratch(&mut self, _: usize) {}
    fn release_scratch_range(&mut self, _: std::ops::Range<usize>) {}
    fn allocation_failure(
        &self,
        error: std::collections::TryReserveError,
        bytes: usize,
        align: usize,
    ) -> Self::Error {
        let _ = (bytes, align);
        self.allocation_error(error)
    }
}
pub struct OrdinaryProjectionAllocation;
impl ProjectionAllocation for OrdinaryProjectionAllocation {
    type Error = crate::DomError;
    fn reserve(&mut self, _: usize) -> Result<(), Self::Error> {
        Ok(())
    }
    fn allocation_error(&self, _: std::collections::TryReserveError) -> Self::Error {
        std::alloc::handle_alloc_error(std::alloc::Layout::new::<u8>())
    }
    fn allocation_failure(
        &self,
        _: std::collections::TryReserveError,
        bytes: usize,
        align: usize,
    ) -> Self::Error {
        match std::alloc::Layout::from_size_align(bytes.max(1), align) {
            Ok(layout) => std::alloc::handle_alloc_error(layout),
            Err(_) => panic!("DOM projection capacity overflow"),
        }
    }
}
pub(crate) fn admitted_push<T, A: ProjectionAllocation>(
    output: &mut Vec<T>,
    value: T,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), crate::DomError>,
) -> Result<(), A::Error> {
    check().map_err(A::Error::from)?;
    let needed = output
        .len()
        .checked_add(1)
        .ok_or_else(|| A::Error::from(crate::DomError::SemanticProjectionCapacity))?;
    reserve_vector(output, needed, allocation)?;
    output.push(value);
    Ok(())
}
pub(crate) fn admitted_append<A: ProjectionAllocation>(
    output: &mut String,
    value: &str,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), crate::DomError>,
) -> Result<(), A::Error> {
    check().map_err(A::Error::from)?;
    let needed = output
        .len()
        .checked_add(value.len())
        .ok_or_else(|| A::Error::from(crate::DomError::SemanticProjectionCapacity))?;
    reserve_string(output, needed, allocation)?;
    for ch in value.chars() {
        check().map_err(A::Error::from)?;
        output.push(ch);
    }
    Ok(())
}

pub(crate) fn growth_capacity(current: usize, needed: usize) -> Result<usize, crate::DomError> {
    if needed <= current {
        return Ok(current);
    }
    let doubled = current.checked_mul(2).unwrap_or(needed);
    let planned = doubled.max(needed).max(4);
    if planned > isize::MAX as usize {
        return Err(crate::DomError::SemanticProjectionCapacity);
    }
    Ok(planned)
}
pub(crate) fn checked_capacity_bytes<T>(capacity: usize) -> Result<usize, crate::DomError> {
    capacity
        .checked_mul(std::mem::size_of::<T>())
        .filter(|bytes| *bytes <= isize::MAX as usize)
        .ok_or(crate::DomError::SemanticProjectionCapacity)
}

#[cfg(test)]
mod growth_controls {
    use super::*;
    struct Allocation {
        calls: usize,
        bytes: usize,
    }
    impl ProjectionAllocation for Allocation {
        type Error = crate::DomError;
        fn reserve(&mut self, bytes: usize) -> Result<(), Self::Error> {
            self.calls += 1;
            self.bytes += bytes;
            Ok(())
        }
        fn allocation_error(&self, _: std::collections::TryReserveError) -> Self::Error {
            crate::DomError::SemanticProjectionCapacity
        }
    }
    #[test]
    fn long_character_and_vector_growth_is_geometric() {
        let mut allocation = Allocation { calls: 0, bytes: 0 };
        let mut text = String::new();
        for _ in 0..65536 {
            admitted_append(&mut text, "x", &mut allocation, &mut || Ok(())).unwrap();
        }
        assert_eq!(text.len(), 65536);
        assert!(allocation.calls <= 16);
        assert!(allocation.bytes >= text.len());
        let mut values = Vec::new();
        let before = allocation.calls;
        for value in 0..65536u32 {
            admitted_push(&mut values, value, &mut allocation, &mut || Ok(())).unwrap();
        }
        assert_eq!(values.len(), 65536);
        assert!(allocation.calls - before <= 16);
        assert!(growth_capacity(0, usize::MAX).is_err());
        assert!(checked_capacity_bytes::<[u8; 4096]>(isize::MAX as usize / 4096 + 1).is_err());
        assert_eq!(checked_capacity_bytes::<[u8; 4096]>(4).unwrap(), 16384);
    }
}

/// Shared geometric reservation engine for DOM and layout producers. It does
/// not execute traversal checkpoints; caller preserves its checkpoint error.
pub fn reserve_vector<T, A: ProjectionAllocation>(
    output: &mut Vec<T>,
    needed: usize,
    allocation: &mut A,
) -> Result<(), A::Error> {
    let capacity = growth_capacity(output.capacity(), needed).map_err(A::Error::from)?;
    if capacity == output.capacity() {
        return Ok(());
    }
    let total = checked_capacity_bytes::<T>(capacity).map_err(A::Error::from)?;
    let previous = checked_capacity_bytes::<T>(output.capacity()).map_err(A::Error::from)?;
    if total - previous >= 1024 * 1024
        && std::env::var("ORYN_NATIVE_ALLOCATION_DIAGNOSTICS").ok().as_deref() == Some("1")
    {
        static EMITTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        if EMITTED.fetch_update(std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed, |n| (n < 16).then_some(n + 1)).is_ok()
        {
            eprintln!("oryn-projection-vector-growth type={} len={} capacity={} next_capacity={} delta_bytes={}",
                std::any::type_name::<T>(), output.len(), output.capacity(), capacity, total - previous);
        }
    }
    allocation.reserve(total - previous)?;
    output
        .try_reserve_exact(capacity - output.len())
        .map_err(|error| allocation.allocation_failure(error, total, std::mem::align_of::<T>()))?;
    Ok(())
}
pub fn reserve_string<A: ProjectionAllocation>(
    output: &mut String,
    needed: usize,
    allocation: &mut A,
) -> Result<(), A::Error> {
    let capacity = growth_capacity(output.capacity(), needed).map_err(A::Error::from)?;
    if capacity == output.capacity() {
        return Ok(());
    }
    allocation.reserve(capacity - output.capacity())?;
    output
        .try_reserve_exact(capacity - output.len())
        .map_err(|error| allocation.allocation_failure(error, capacity, 1))?;
    Ok(())
}
