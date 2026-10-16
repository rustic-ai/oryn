//! Shared completion-action metadata. Copies of this owner do not clone strings.
use super::*;
use crate::idb_owned_admission::{Buffer, Lease};
pub(super) struct OwnedAction {
    value: IdbCompletionAction,
    // Payload destructor must run before its exact allocation leases.
    _custody: Option<Buffer<Lease>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_rejects_string_payload_and_shares_identity() {
        assert!(
            OwnedAction::control(IdbCompletionAction::Open {
                name: String::new()
            })
            .is_err()
        );
        let value = OwnedAction::unit();
        let copy = value.clone();
        assert!(Arc::ptr_eq(&value, &copy));
        assert!(matches!(copy.value(), IdbCompletionAction::Unit));
    }
    #[test]
    fn retired_empty_string_action_copy_rejects_before_allocation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(html::parse("<body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.retire_execution();
        let before = state.idb_payload_ledger.live_bytes();
        assert!(matches!(
            OwnedAction::copy(
                scope,
                &state,
                &IdbCompletionAction::Open {
                    name: String::new()
                }
            ),
            Err(V8BindingError::JavaScriptException)
        ));
        assert_eq!(state.idb_payload_ledger.live_bytes(), before);
    }
}
impl OwnedAction {
    #[cfg(test)]
    pub(super) fn control(value: IdbCompletionAction) -> Result<Arc<Self>, V8BindingError> {
        if matches!(
            &value,
            IdbCompletionAction::Open { .. }
                | IdbCompletionAction::DeleteDatabase { .. }
                | IdbCompletionAction::Batch { .. }
                | IdbCompletionAction::Cursor { .. }
        ) {
            return Err(V8BindingError::Allocation(
                "IDB string action requires admission",
            ));
        }
        Ok(Arc::new(Self {
            value,
            _custody: None,
        }))
    }
    pub(super) fn unit() -> Arc<Self> {
        Arc::new(Self {
            value: IdbCompletionAction::Unit,
            _custody: None,
        })
    }
    pub(super) fn value(&self) -> &IdbCompletionAction {
        &self.value
    }
    pub(super) fn copy(
        scope: &mut v8::PinScope,
        state: &V8BindingState,
        value: &IdbCompletionAction,
    ) -> Result<Arc<Self>, V8BindingError> {
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        let heap = scope.get_heap_statistics();
        let used = heap
            .used_heap_size()
            .checked_add(heap.external_memory())
            .ok_or(V8BindingError::Allocation("IDB action heap"))?;
        let ledger = &state.idb_payload_ledger;
        let limit = heap.heap_size_limit();
        let count = usize::from(matches!(
            value,
            IdbCompletionAction::Open { .. }
                | IdbCompletionAction::DeleteDatabase { .. }
                | IdbCompletionAction::Batch { .. }
                | IdbCompletionAction::Cursor { .. }
        ));
        let mut custody = Buffer::allocate(ledger, count, limit, used)
            .map_err(|_| V8BindingError::Allocation("IDB action custody"))?;
        let mut string = |value: &str| -> Result<String, V8BindingError> {
            let text = owned_names::Names::copy_text(value, ledger, limit, used, &mut || {
                scope.is_execution_terminating() || state.execution_retired()
            })
            .map_err(|_| {
                if scope.is_execution_terminating() || state.execution_retired() {
                    V8BindingError::JavaScriptException
                } else {
                    V8BindingError::Allocation("IDB action string")
                }
            })?;
            let (value, lease) = text.into_parts();
            custody
                .push(lease)
                .map_err(|_| V8BindingError::Allocation("IDB action custody"))?;
            Ok(value)
        };
        let value = match value {
            IdbCompletionAction::Open { name } => IdbCompletionAction::Open {
                name: string(name)?,
            },
            IdbCompletionAction::DeleteDatabase { name } => IdbCompletionAction::DeleteDatabase {
                name: string(name)?,
            },
            IdbCompletionAction::Batch {
                count,
                direction,
                kind,
            } => IdbCompletionAction::Batch {
                count: *count,
                direction: string(direction)?,
                kind: *kind,
            },
            IdbCompletionAction::Cursor {
                source_id,
                direction,
                query,
                with_value,
            } => IdbCompletionAction::Cursor {
                source_id: *source_id,
                direction: string(direction)?,
                query: query.clone(),
                with_value: *with_value,
            },
            IdbCompletionAction::Value => IdbCompletionAction::Value,
            IdbCompletionAction::Key => IdbCompletionAction::Key,
            IdbCompletionAction::Count => IdbCompletionAction::Count,
            IdbCompletionAction::CursorRefresh { cursor_id } => {
                IdbCompletionAction::CursorRefresh {
                    cursor_id: *cursor_id,
                }
            }
            IdbCompletionAction::OpenFinalized {
                request_id,
                database_id,
                transaction_id,
            } => IdbCompletionAction::OpenFinalized {
                request_id: *request_id,
                database_id: *database_id,
                transaction_id: *transaction_id,
            },
            IdbCompletionAction::TransactionFinalized {
                transaction_id,
                event_type,
            } => IdbCompletionAction::TransactionFinalized {
                transaction_id: *transaction_id,
                event_type,
            },
            IdbCompletionAction::Unit => IdbCompletionAction::Unit,
        };
        // Shared owner control allocation follows the existing queue Arc policy;
        // this method qualifies actual action string payloads only.
        if scope.is_execution_terminating() || state.execution_retired() {
            return Err(V8BindingError::JavaScriptException);
        }
        Ok(Arc::new(Self {
            value,
            _custody: Some(custody),
        }))
    }
}
