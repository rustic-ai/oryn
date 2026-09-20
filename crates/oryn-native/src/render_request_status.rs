//! Shared status for one native resource request, frozen when its response ends.
//! Joining consumers may promote an in-flight preload, never a completed one.
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
#[derive(Clone, Default)]
pub(crate) struct Status(Arc<AtomicU8>);
impl Status {
    #[cfg(feature = "v8-host")]
    pub(crate) fn new(blocking: bool) -> Self {
        Self(Arc::new(AtomicU8::new(u8::from(blocking))))
    }
    pub(crate) fn promote(&self) {
        let _ = self
            .0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
    }
    #[cfg(test)]
    pub(crate) fn blocking(&self) -> bool {
        self.0.load(Ordering::Acquire) & 1 != 0
    }
    pub(crate) fn finish(&self) -> bool {
        self.0.fetch_or(2, Ordering::AcqRel) & 1 != 0
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_a_consumer_before_response_completion_promotes_status() {
        let early = Status::default();
        let consumer = early.clone();
        consumer.promote();
        assert!(early.finish());
        assert!(consumer.finish());
        let late = Status::default();
        assert!(!late.finish());
        late.promote();
        assert!(!late.blocking());
        assert!(!late.finish());
    }
}
