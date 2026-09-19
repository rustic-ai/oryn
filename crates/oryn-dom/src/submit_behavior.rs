//! Native submit behavior state shared by activation, validation, and semantics.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitButtonBehavior {
    pub disabled: bool,
    pub form_no_validate: bool,
    pub form_action: String,
    pub form_method: String,
    pub form_enctype: String,
    pub form_target: String,
    pub name: String,
    pub value: String,
    pub(crate) role: String,
}
impl Default for SubmitButtonBehavior {
    fn default() -> Self {
        Self {
            disabled: false,
            form_no_validate: false,
            form_action: String::new(),
            form_method: String::new(),
            form_enctype: String::new(),
            form_target: String::new(),
            name: String::new(),
            value: String::new(),
            role: "button".into(),
        }
    }
}
impl DomRuntime {
    pub fn set_submit_behavior(
        &mut self,
        node: NodeId,
        value: SubmitButtonBehavior,
    ) -> Result<(), DomError> {
        let record = self.arena.get_mut(node)?;
        if record.submit_behavior.as_ref() != Some(&value) {
            record.submit_behavior = Some(value);
            self.mutations.push(Mutation::FormState { node });
        }
        Ok(())
    }
    pub fn submitter_parameter(&self, node: NodeId, name: &str) -> Result<Option<&str>, DomError> {
        if let Some(behavior) = &self.arena.get(node)?.submit_behavior {
            let value = match name {
                "formaction" => &behavior.form_action,
                "formmethod" => &behavior.form_method,
                "formenctype" => &behavior.form_enctype,
                "formtarget" => &behavior.form_target,
                _ => return Ok(None),
            };
            return Ok((!value.is_empty()).then_some(value.as_str()));
        }
        self.attribute_in_null_namespace(node, name)
    }
    pub fn submitter_no_validate(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(match &self.arena.get(node)?.submit_behavior {
            Some(behavior) => behavior.form_no_validate,
            None => self
                .attribute_in_null_namespace(node, "formnovalidate")?
                .is_some(),
        })
    }
    pub fn submitter_value(&self, node: NodeId) -> Result<String, DomError> {
        match &self.arena.get(node)?.submit_behavior {
            Some(behavior) => Ok(behavior.value.clone()),
            None => self.control_value(node),
        }
    }
}
