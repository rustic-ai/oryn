//! Native OS file-chooser request; no path or file authority comes from markup.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FilePicker {
    pub session: u64,
    pub accept: String,
    pub multiple: bool,
}
impl DomRuntime {
    pub fn open_file_picker(&mut self, node: NodeId) -> Result<bool, DomError> {
        if !self.is_file_input(node)
            || !self.is_connected(node)?
            || self.control_disabled(node)?
            || self.is_inert(node)?
        {
            return Ok(false);
        }
        let root = self.node_document(node)?.unwrap_or(self.document());
        let previous = self
            .arena
            .descendants_inclusive(root)?
            .into_iter()
            .filter(|node| {
                self.arena
                    .get(*node)
                    .is_ok_and(|node| node.file_picker.is_some())
            })
            .collect::<Vec<_>>();
        for previous in previous {
            self.close_file_picker(previous)?;
        }
        let accept = self
            .attribute_in_null_namespace(node, "accept")?
            .unwrap_or("")
            .chars()
            .take(4096)
            .collect();
        let multiple = self
            .attribute_in_null_namespace(node, "multiple")?
            .is_some();
        let session = self.mutation_cursor().0;
        self.arena.get_mut(node)?.file_picker = Some(FilePicker {
            session,
            accept,
            multiple,
        });
        self.record_mutation(Mutation::FormState { node });
        Ok(true)
    }
    pub fn file_picker_snapshot(&self, node: NodeId) -> Result<Option<FilePicker>, DomError> {
        if !self.is_file_input(node)
            || !self.is_connected(node)?
            || self.control_disabled(node)?
            || self.is_inert(node)?
        {
            return Ok(None);
        }
        let Some(mut request) = self.arena.get(node)?.file_picker.clone() else {
            return Ok(None);
        };
        request.multiple = self
            .attribute_in_null_namespace(node, "multiple")?
            .is_some();
        request.accept = self
            .attribute_in_null_namespace(node, "accept")?
            .unwrap_or("")
            .chars()
            .take(4096)
            .collect();
        Ok(Some(request))
    }
    pub fn close_file_picker(&mut self, node: NodeId) -> Result<(), DomError> {
        if self.arena.get_mut(node)?.file_picker.take().is_some() {
            self.record_mutation(Mutation::FormState { node });
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_picker_requests_are_native_bounded_and_supersede_prior_owners() {
        let mut dom = html::parse(
            "<input id=a type=file multiple accept='.txt,image/png'><input id=b type=file>",
        )
        .into_runtime()
        .unwrap();
        let a = dom.find_by_id("a").unwrap();
        let b = dom.find_by_id("b").unwrap();
        dom.open_file_picker(a).unwrap();
        let request = dom.file_picker_snapshot(a).unwrap().unwrap();
        assert!(request.multiple);
        assert_eq!(request.accept, ".txt,image/png");
        dom.open_file_picker(b).unwrap();
        assert!(dom.file_picker_snapshot(a).unwrap().is_none());
        dom.set_attribute(b, "disabled", "").unwrap();
        assert!(dom.file_picker_snapshot(b).unwrap().is_none());
        let clone = dom.clone_node(b, false).unwrap();
        assert!(dom.arena().get(clone).unwrap().file_picker.is_none());
    }
}
