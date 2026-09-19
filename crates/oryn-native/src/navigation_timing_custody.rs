//! Document timing credit uses the same owner as queued sources, but survives
//! queue release through the shared timing record (including parked documents).
use std::sync::{Arc, atomic::AtomicUsize};
#[derive(Clone, Debug, Default)]
pub(crate) struct Budget(pub(crate) Arc<AtomicUsize>);
impl oryn_js::navigation_timing::MetadataLease for crate::executable::ScriptSourceReservation {}
impl oryn_js::navigation_timing::MetadataBudget for Budget {
    fn identity(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
    fn reserve(&self, bytes: usize) -> Option<Arc<dyn oryn_js::navigation_timing::MetadataLease>> {
        crate::executable::ScriptSourceReservation::reserve(&self.0, bytes)
            .map(|lease| Arc::new(lease) as Arc<dyn oryn_js::navigation_timing::MetadataLease>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_js::navigation_timing::{Kind, NavigationTiming, Stage};
    use std::sync::atomic::Ordering;
    #[test]
    fn parked_clones_keep_original_budget_until_last_document_owner_retires() {
        let budget = Budget::default();
        let now = std::time::Instant::now();
        let record = NavigationTiming::new(now, Kind::Navigate);
        let name = format!("https://example.test/{}", "x".repeat(131072));
        record.set_name(name.clone()).unwrap();
        record.admit_metadata(Arc::new(budget.clone())).unwrap();
        let admitted = budget.0.load(Ordering::Acquire);
        assert!(admitted > name.len());
        let cached = record.clone();
        drop(record);
        assert_eq!(budget.0.load(Ordering::Acquire), admitted);
        assert_eq!(cached.snapshot().name, name);
        cached.mark(Stage::Activation, now);
        cached.finish(now, 7);
        assert_eq!(budget.0.load(Ordering::Acquire), admitted);
        let other = Budget::default();
        assert!(cached.admit_metadata(Arc::new(other.clone())).is_err());
        assert_eq!(other.0.load(Ordering::Acquire), 0);
        drop(cached);
        assert_eq!(budget.0.load(Ordering::Acquire), 0);
    }
    #[test]
    fn response_queue_and_document_metadata_require_independent_credit() {
        let budget = Budget::default();
        let held =
            crate::executable::ScriptSourceReservation::reserve(&budget.0, 32 * 1024 * 1024 - 1024)
                .unwrap();
        let record = NavigationTiming::new(std::time::Instant::now(), Kind::Navigate);
        record.set_name("https://example.test/".into()).unwrap();
        assert!(record.admit_metadata(Arc::new(budget.clone())).is_err());
        assert_eq!(budget.0.load(Ordering::Acquire), 32 * 1024 * 1024 - 1024);
        drop(held);
        record.admit_metadata(Arc::new(budget.clone())).unwrap();
        assert!(budget.0.load(Ordering::Acquire) > 0);
        drop(record);
        assert_eq!(budget.0.load(Ordering::Acquire), 0);
    }
    #[test]
    fn separator_heavy_server_timing_reserves_and_releases_parser_scratch() {
        let budget = Budget::default();
        let now = std::time::Instant::now();
        let record = NavigationTiming::new(now, Kind::Navigate);
        record.set_name("https://example.test/old".into()).unwrap();
        record.admit_metadata(Arc::new(budget.clone())).unwrap();
        let admitted = budget.0.load(Ordering::Acquire);
        let occupied = crate::executable::ScriptSourceReservation::reserve(
            &budget.0,
            32 * 1024 * 1024 - admitted - 2 * 1024 * 1024,
        )
        .unwrap();
        let headers = vec![("server-timing".into(), ",".repeat(65536))];
        assert_eq!(
            record
                .response(
                    "https://example.test/new".into(),
                    200,
                    &headers,
                    now,
                    now,
                    None,
                    false
                )
                .unwrap_err(),
            "navigation timing scratch capacity"
        );
        assert_eq!(record.snapshot().name, "https://example.test/old");
        drop(occupied);
        record
            .response(
                "https://example.test/new".into(),
                200,
                &headers,
                now,
                now,
                None,
                false,
            )
            .unwrap();
        assert!(
            budget.0.load(Ordering::Acquire) < 1024 * 1024,
            "temporary split vectors do not remain charged to document"
        );
        drop(record);
        assert_eq!(budget.0.load(Ordering::Acquire), 0);
    }
    #[test]
    fn postadmission_metadata_failure_preserves_previous_record_and_releases_candidate() {
        let budget = Budget::default();
        let now = std::time::Instant::now();
        let record = NavigationTiming::new(now, Kind::Navigate);
        record.set_name("https://example.test/old".into()).unwrap();
        record.admit_metadata(Arc::new(budget.clone())).unwrap();
        let admitted = budget.0.load(Ordering::Acquire);
        let occupied = crate::executable::ScriptSourceReservation::reserve(
            &budget.0,
            32 * 1024 * 1024 - admitted,
        )
        .unwrap();
        assert!(
            record
                .response(
                    "https://example.test/new".into(),
                    200,
                    &[],
                    now,
                    now,
                    None,
                    false
                )
                .is_err()
        );
        assert_eq!(record.snapshot().name, "https://example.test/old");
        assert!(
            record
                .unload("https://example.test".into(), now, now)
                .is_err()
        );
        assert_eq!(budget.0.load(Ordering::Acquire), 32 * 1024 * 1024);
        drop(occupied);
        record
            .unload("https://example.test".into(), now, now)
            .unwrap();
        assert!(budget.0.load(Ordering::Acquire) > admitted);
        drop(record);
        assert_eq!(budget.0.load(Ordering::Acquire), 0);
    }
}
