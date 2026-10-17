//! Temporary PI attribute scratch is charged to the selected native owner.
use super::*;
struct Receipt {
    _payload: crate::idb_owned_admission::Lease,
    _metadata: Option<crate::idb_owned_admission::Lease>,
}
pub(super) struct Admission {
    ledger: std::sync::Arc<crate::idb_owned_admission::Ledger>,
    receipt: crate::idb_owned_admission::HeapReceipt,
    leases: Vec<Receipt>,
}
struct Metadata<'a> {
    ledger: &'a std::sync::Arc<crate::idb_owned_admission::Ledger>,
    receipt: &'a crate::idb_owned_admission::HeapReceipt,
    lease: Option<crate::idb_owned_admission::Lease>,
}
impl oryn_dom::projection_allocation::ProjectionAllocation for Metadata<'_> {
    type Error = V8BindingError;
    fn reserve(&mut self, bytes: usize) -> Result<(), Self::Error> {
        self.lease = Some(
            self.ledger
                .reserve_from_receipt(bytes, self.receipt)
                .map_err(V8BindingError::Message)?,
        );
        Ok(())
    }
    fn allocation_error(&self, _: std::collections::TryReserveError) -> Self::Error {
        V8BindingError::Allocation("PI scratch metadata allocation")
    }
}
impl oryn_dom::projection_allocation::ProjectionAllocation for Admission {
    type Error = V8BindingError;
    fn reserve(&mut self, bytes: usize) -> Result<(), Self::Error> {
        let payload = self
            .ledger
            .reserve_from_receipt(bytes, &self.receipt)
            .map_err(V8BindingError::Message)?;
        let needed = self
            .leases
            .len()
            .checked_add(1)
            .ok_or(V8BindingError::Allocation("PI receipt capacity"))?;
        let mut metadata = Metadata {
            ledger: &self.ledger,
            receipt: &self.receipt,
            lease: None,
        };
        oryn_dom::projection_allocation::reserve_vector(&mut self.leases, needed, &mut metadata)?;
        self.leases.push(Receipt {
            _payload: payload,
            _metadata: metadata.lease,
        });
        Ok(())
    }
    fn allocation_error(&self, _: std::collections::TryReserveError) -> Self::Error {
        V8BindingError::Allocation("PI attribute allocation")
    }
}
impl Admission {
    pub(super) fn new(
        scope: &mut v8::PinScope,
        state: &V8BindingState,
    ) -> Result<Self, V8BindingError> {
        let heap = scope.get_heap_statistics();
        let ledger = state.idb_payload_ledger.clone();
        let receipt = ledger
            .heap_receipt_with_statistics(
                heap.heap_size_limit(),
                heap.used_heap_size(),
                heap.external_memory(),
            )
            .map_err(V8BindingError::Message)?;
        Ok(Self {
            ledger,
            receipt,
            leases: Vec::new(),
        })
    }
}
