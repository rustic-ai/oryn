// A ready promise may settle only after the embedding has committed layout for
// the exact font demand generation. Completing a fetch alone is insufficient.
pub use super::font_loading_transport::FontDemandStamp;
#[derive(Default)]
pub struct LayoutReadiness {
    demanded: Option<FontDemandStamp>,
    committed: Option<FontDemandStamp>,
    retired: bool,
}
impl LayoutReadiness {
    pub fn demand(&mut self, stamp: FontDemandStamp) -> bool {
        if self.retired { return false; }
        if self.demanded != Some(stamp) {
            self.demanded = Some(stamp);
            self.committed = None;
        }
        true
    }
    pub fn commit(&mut self, stamp: FontDemandStamp) -> bool {
        if self.retired || self.demanded != Some(stamp) { return false; }
        self.committed = Some(stamp);
        true
    }
    pub fn ready(&self, owner: super::font_loading_transport::FontLoadOwner) -> bool {
        !self.retired && self.demanded.is_some_and(|stamp| stamp.owner == owner)
            && self.committed == self.demanded
    }
    pub fn retire(&mut self) { self.retired = true; self.committed = None; }
    pub fn stamp(&self)->Option<FontDemandStamp>{self.committed}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fetch_terminal_does_not_replace_layout_commit() {
        let owner = super::super::font_loading_transport::FontLoadOwner {
            instance: 1, document: oryn_dom::NodeId {slot:0,generation:0}, generation: 1, epoch: 1,
        };
        let first = FontDemandStamp { owner, demand: 1 };
        let second = FontDemandStamp { owner, demand: 2 };
        let mut readiness = LayoutReadiness::default();
        assert!(!readiness.ready(owner));
        assert!(readiness.demand(first));
        assert!(!readiness.ready(owner));
        assert!(readiness.commit(first));
        assert!(readiness.ready(owner));
        assert!(readiness.demand(second));
        assert!(!readiness.commit(first));
        assert!(!readiness.ready(owner));
        assert!(readiness.commit(second));
        readiness.retire();
        assert!(!readiness.ready(owner));
        assert!(!readiness.commit(second));
    }
}
