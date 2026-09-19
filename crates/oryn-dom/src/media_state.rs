//! Native media states are independent of authored attributes and CSS matches.
use crate::*;
impl DomRuntime {
    pub fn media_selector_state(&self, node: NodeId) -> Result<u8, DomError> {
        let record = self.arena.get(node)?;
        Ok(
            if self.is_element(node, "http://www.w3.org/1999/xhtml", "audio")
                || self.is_element(node, "http://www.w3.org/1999/xhtml", "video")
            {
                record.media_selector_state.unwrap_or(2) | 1
            } else {
                0
            },
        )
    }
    pub fn set_media_selector_state(&mut self, node: NodeId, value: u8) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.media_selector_state != Some(value) {
            record.media_selector_state = Some(value);
            self.mutations.push(Mutation::FormState { node });
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paused_default_and_live_flags_are_media_only_and_not_cloned() {
        let mut dom = DomRuntime::new();
        let video = dom.create_element("video");
        let div = dom.create_element("div");
        assert_eq!(dom.media_selector_state(video).unwrap(), 3);
        assert_eq!(dom.media_selector_state(div).unwrap(), 0);
        dom.set_media_selector_state(video, 4 | 8).unwrap();
        assert_eq!(dom.media_selector_state(video).unwrap(), 13);
        let cloned = dom.clone_node(video, false).unwrap();
        assert_eq!(dom.media_selector_state(cloned).unwrap(), 3);
    }
}
