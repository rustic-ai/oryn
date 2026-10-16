//! Explicit native-libtest diagnosis only: no authored keys/values/errors.
use std::sync::atomic::{AtomicUsize,Ordering};
#[derive(Clone,Copy)]
pub(super) enum Stage { Submit, Busy, Delivery, Delivered, Suspended, Failure,
    Prepare, Publish, UpgradeEvent, SuccessEvent, ErrorEvent, FinalizeOpen,
    FinalizeTransaction, Finish, Flush, Poll, Checkpoint, Other }
impl Stage {
    fn label(self)->&'static str {match self {
        Self::Submit=>"submit",Self::Busy=>"busy",Self::Delivery=>"delivery",
        Self::Delivered=>"delivered",Self::Suspended=>"suspended",Self::Failure=>"failure",
        Self::Prepare=>"prepare",Self::Publish=>"publish",Self::UpgradeEvent=>"upgrade_event",
        Self::SuccessEvent=>"success_event",Self::ErrorEvent=>"error_event",
        Self::FinalizeOpen=>"finalize_open",Self::FinalizeTransaction=>"finalize_transaction",
        Self::Finish=>"finish",Self::Flush=>"flush",Self::Poll=>"poll",
        Self::Checkpoint=>"checkpoint",Self::Other=>"other"}}
}
fn admit(enabled:bool,count:&AtomicUsize)->bool {
    enabled&&count.fetch_update(Ordering::Relaxed,Ordering::Relaxed,
        |n|(n<512).then_some(n+1)).is_ok()
}
pub(super) fn record(stage:Stage,request:u64,related:u64) {
    static COUNT:AtomicUsize=AtomicUsize::new(0);
    if !admit(std::env::var("ORYN_NATIVE_IDB_STAGE_DIAGNOSTICS").ok().as_deref()==Some("1"),&COUNT) {return;}
    eprintln!("oryn-native-idb-stage stage={} request={request} related={related}",stage.label());
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn explicit_diagnostic_disabled_and_bounded() {
        let count=AtomicUsize::new(0);
        assert!(!admit(false,&count));assert_eq!(count.load(Ordering::Relaxed),0);
        for _ in 0..512 {assert!(admit(true,&count));}
        assert!(!admit(true,&count));assert_eq!(count.load(Ordering::Relaxed),512);
        assert_eq!(Stage::Failure.label(),"failure");
    }
}
