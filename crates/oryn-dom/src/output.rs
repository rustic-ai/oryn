//! Output value/default mode is native state, independent of authored properties.
use super::*;
impl DomRuntime {
    pub fn output_value(&self, node: NodeId) -> Result<String, DomError> {
        self.arena.text_content(node)
    }
    pub fn output_default_value(&self, node: NodeId) -> Result<String, DomError> {
        match &self.arena.get(node)?.output_default {
            Some(value) => Ok(value.clone()),
            None => self.output_value(node),
        }
    }
    pub fn set_output_value(&mut self, node: NodeId, value: String) -> Result<(), DomError> {
        let previous = self.arena.get(node)?.output_default.clone();
        if previous.is_none() {
            let default = self.output_value(node)?;
            self.arena.get_mut(node)?.output_default = Some(default);
        }
        if let Err(error) = self.set_text_content(node, value) {
            self.arena.get_mut(node)?.output_default = previous;
            return Err(error);
        }
        Ok(())
    }
    pub fn set_output_default_value(
        &mut self,
        node: NodeId,
        value: String,
    ) -> Result<(), DomError> {
        if self.arena.get(node)?.output_default.is_some() {
            self.arena.get_mut(node)?.output_default = Some(value);
            Ok(())
        } else {
            self.set_text_content(node, value)
        }
    }
    pub fn reset_output(&mut self, node: NodeId) -> Result<(), DomError> {
        let value = self.output_default_value(node)?;
        self.set_text_content(node, value)?;
        self.arena.get_mut(node)?.output_default = None;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_mode_follows_descendant_text_reset_clone_and_native_import() {
        let mut dom = html::parse("<output id=o><span>default</span></output>")
            .into_runtime()
            .unwrap();
        let o = dom.find_by_id("o").unwrap();
        assert_eq!(dom.output_default_value(o).unwrap(), "default");
        dom.set_output_value(o, "changed".into()).unwrap();
        dom.set_text_content(o, "authored").unwrap();
        assert_eq!(dom.output_value(o).unwrap(), "authored");
        assert_eq!(dom.output_default_value(o).unwrap(), "default");
        dom.set_output_default_value(o, "reset".into()).unwrap();
        assert_eq!(dom.output_value(o).unwrap(), "authored");
        let cloned = dom.clone_node(o, true).unwrap();
        assert_eq!(dom.output_default_value(cloned).unwrap(), "authored");
        let mut foreign = html::parse("").into_runtime().unwrap();
        let native = foreign.import_subtree(dom.arena(), o).unwrap();
        assert_eq!(foreign.output_default_value(native).unwrap(), "reset");
        let imported = foreign.import_node_from(dom.arena(), o, true).unwrap();
        assert_eq!(foreign.output_default_value(imported).unwrap(), "authored");
        dom.reset_output(o).unwrap();
        assert_eq!(dom.output_value(o).unwrap(), "reset");
        dom.set_text_content(o, "new default").unwrap();
        assert_eq!(dom.output_default_value(o).unwrap(), "new default");
    }
}
