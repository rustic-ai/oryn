//! Agent task draining and checkpoint scheduling for the raw V8 host.

#[cfg(test)]
use super::FAIL_FETCH_DELIVERY_ONCE;
#[cfg(unix)]
use super::publish_local_window_inventory;
use super::{
    AgentExecution, AsyncFetches, ExecutionDeadline, FrameLifetimes, HostError,
    NativeRealmIdentities, RawV8Host, RealmEndpointRegistry, RealmId, RuntimeBootstrapPhase,
    RuntimeBootstrapped, Task, TaskSource, TimerRegistry, TimerTaskNesting, deliver_control_event,
    deliver_mutation_observers, deliver_selection_change, fixup_focus_eligibility,
    has_pending_mutation_observer_microtask, nested_timer_delay, queue_mutation_observer_microtask,
    report_timer_exception, reset_mutation_observer_microtask_after_termination,
};
use std::{
    collections::BTreeSet,
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

// A binding instance and every mutable document epoch accompany its DOM cursor.
// A cursor alone cannot distinguish replacement or a child realm created mid-pass.
type BindingCheckpointIdentity = oryn_js::v8_bindings::ExecutableCheckpointIdentity;
pub(crate) type ExecutableCheckpointWitness = Vec<(RealmId, BindingCheckpointIdentity)>;

impl RawV8Host {
    /// Observe the selected root's complete live realm membership without
    /// adopting, draining or polling anything. Unsettled ownership is not idle.
    fn checkpoint_realms(&self) -> Option<Vec<RealmId>> {
        let identities = self.isolate.get_slot::<NativeRealmIdentities>()?;
        if !identities.pending_contexts.is_empty() || self.realm_is_parked(self.active_realm) {
            return None;
        }
        let mut realms = self.active_root_realm_ids();
        let frames = self.isolate.get_slot::<FrameLifetimes>();
        if let Some(frames) = frames {
            let root = frames
                .live
                .values()
                .find(|frame| frame.environment.get().realm == self.active_realm)
                .map(|frame| frame.root_realm)
                .unwrap_or(self.active_realm);
            if frames.discarded_realms.iter().any(|realm| {
                frames
                    .discarded_owners
                    .get(realm)
                    .is_none_or(|owner| *owner == root)
            }) {
                return None;
            }
            let endpoints = self.isolate.get_slot::<RealmEndpointRegistry>();
            if frames.creating.iter().any(|(parent, _)| {
                endpoints
                    .and_then(|registry| {
                        registry
                            .0
                            .values()
                            .find(|(environment, _)| environment.id == *parent)
                    })
                    .is_none_or(|(environment, _)| realms.contains(&environment.realm))
            }) {
                return None;
            }
        }
        realms.retain(|realm| !self.realm_is_parked(*realm));
        if realms.is_empty() {
            return None;
        }
        for realm in &realms {
            let state = self.binding_states.get(realm)?;
            if !self.contexts.contains_key(realm)
                || state.realm() != *realm
                || state.execution_retired()
                || state.execution_suspended()
                || state.execution_tasks_stopped()
            {
                return None;
            }
        }
        Some(realms)
    }

    /// A non-consuming before/after witness; None always requires another pass.
    pub(crate) fn executable_checkpoint_witness(&self) -> Option<ExecutableCheckpointWitness> {
        self.checkpoint_realms()?
            .into_iter()
            .map(|realm| {
                let state = self.binding_states.get(&realm)?;
                Some((realm, state.executable_checkpoint_identity()?))
            })
            .collect()
    }

    /// Local queues complement, rather than replace, has_runnable_page_work.
    pub(crate) fn has_pending_executable_checkpoint_work(&self, outer_root: RealmId) -> bool {
        // Root history updates are consumed by the outer page owner after
        // this checkpoint returns. Child updates are drained inside the next
        // pass, so a late child publication must still keep the loop alive.
        // Called only after entered callbacks/scopes have returned. The global
        // observer queue also owns notifications not represented by local queues.
        has_pending_mutation_observer_microtask(&self.isolate)
            || self.checkpoint_realms().is_none_or(|realms| {
                realms.into_iter().any(|realm| {
                    self.binding_states.get(&realm).is_none_or(|state| {
                        state.has_pending_executable_checkpoint_work(realm != outer_root)
                    })
                })
            })
    }

    /// Run the agent's pending microtasks without executing a synthetic script
    /// or polling platform tasks. The parser calls this at script end tags.
    pub fn run_microtask_checkpoint(&mut self) -> Result<(), HostError> {
        self.publish_native_resource_timings();
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handle_scope, &mut self.isolate);
        let context = v8::Local::new(
            handle_scope,
            self.contexts
                .get(&self.active_realm)
                .expect("active realm context exists"),
        );
        let scope = &mut v8::ContextScope::new(handle_scope, context);
        v8::tc_scope!(let scope, scope);
        queue_mutation_observer_microtask(scope);
        scope.perform_microtask_checkpoint();
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            return Err(HostError::LimitExceeded {
                resource: "wall time".into(),
            });
        }
        if scope.has_caught() {
            let exception = scope.exception();
            let message = scope.message();
            let detail = message
                .map(|message| message.get(scope).to_rust_string_lossy(scope))
                .unwrap_or_else(|| "microtask checkpoint failed".into());
            scope.reset();
            if let Some(exception) = exception {
                let canceled =
                    oryn_js::v8_bindings::report_script_exception(scope, exception, message, false)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                if !canceled {
                    return Err(HostError::Execution(detail));
                }
            }
        }
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            return Err(HostError::LimitExceeded {
                resource: "wall time".into(),
            });
        }
        #[cfg(unix)]
        publish_local_window_inventory(scope)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        Ok(())
    }

    pub fn run_dom_observer_checkpoint(&mut self) -> Result<usize, HostError> {
        let _stage = crate::native_timing::stage("host.observers");
        self.publish_native_resource_timings();
        #[cfg(unix)]
        self.publish_native_window_inventory()?;
        let Some(dom) = self.active_dom() else {
            return Ok(0);
        };
        let initial_mutations = dom.borrow_mut().drain_mutations();
        // Most rendering checkpoints have no notifications. Do not spawn
        // a deadline thread or enter V8 on that common read-only path.
        if initial_mutations.is_empty()
            && !has_pending_mutation_observer_microtask(&self.isolate)
            && !self
                .binding_states
                .values()
                .any(|state| state.has_pending_selection_change())
        {
            return Ok(0);
        }
        let mut initial_mutations = Some(initial_mutations);
        let _execution_deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        let mut delivered = 0_usize;
        for _ in 0..1024 {
            let Some(dom) = self.active_dom() else {
                crate::native_timing::count("host.observers.delivered", delivered as u64);
                return Ok(delivered);
            };
            let mutations = initial_mutations
                .take()
                .unwrap_or_else(|| dom.borrow_mut().drain_mutations());
            let cursor = dom.borrow().mutation_cursor();
            let realms = self.binding_states.keys().copied().collect::<Vec<_>>();
            for realm in realms {
                let Some(state) = self.binding_states.get(&realm).cloned() else {
                    continue;
                };
                let Some(context) = self.contexts.get(&realm).cloned() else {
                    continue;
                };
                v8::scope!(let handle_scope, &mut self.isolate);
                let context = v8::Local::new(handle_scope, &context);
                let scope = &mut v8::ContextScope::new(handle_scope, context);
                delivered += deliver_mutation_observers(scope, &state, &mutations)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                if _execution_deadline.expired() || scope.is_execution_terminating() {
                    scope.cancel_terminate_execution();
                    reset_mutation_observer_microtask_after_termination(scope);
                    return Err(HostError::LimitExceeded {
                        resource: "wall time".into(),
                    });
                }
                delivered += usize::from(
                    fixup_focus_eligibility(scope, &state)
                        .map_err(|error| HostError::Execution(error.to_string()))?,
                );
                delivered += usize::from(
                    deliver_selection_change(scope, &state)
                        .map_err(|error| HostError::Execution(error.to_string()))?,
                );
                queue_mutation_observer_microtask(scope);
                scope.perform_microtask_checkpoint();
                if _execution_deadline.expired() || scope.is_execution_terminating() {
                    scope.cancel_terminate_execution();
                    reset_mutation_observer_microtask_after_termination(scope);
                    return Err(HostError::LimitExceeded {
                        resource: "wall time".into(),
                    });
                }
            }
            dom.borrow_mut()
                .release_mutation_roots(&mutations)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            if crate::native_timing::enabled() {
                // Observe only after exact lease acknowledgement has succeeded.
                let realm = self.active_realm;
                if let Some(state) = self.binding_states.get(&realm)
                    && !self.realm_is_parked(realm)
                    && !state.execution_retired()
                    && !state.execution_suspended()
                    && !state.execution_tasks_stopped()
                    && std::rc::Rc::ptr_eq(&state.dom(), &dom)
                    && let Some(context) = self.contexts.get(&realm)
                {
                    v8::scope!(let handles, &mut self.isolate);
                    let context = v8::Local::new(handles, context);
                    let scope = &mut v8::ContextScope::new(handles, context);
                    oryn_js::v8_bindings::publish_native_binding_memory(scope, "post_removal_release");
                }
            }
            if dom.borrow().mutation_cursor() == cursor {
                crate::native_timing::count("host.observers.delivered", delivered as u64);
                return Ok(delivered);
            }
        }
        Err(HostError::LimitExceeded {
            resource: "mutation observer checkpoint iterations".into(),
        })
    }

    pub(crate) fn next_rendering_delay(&self) -> Option<Duration> {
        self.active_root_realm_ids()
            .into_iter()
            .filter(|realm| !self.realm_is_parked(*realm))
            .filter_map(|realm| {
                self.binding_states
                    .get(&realm)
                    .and_then(|state| state.next_rendering_delay())
            })
            .min()
    }

    pub fn next_timer_delay(&mut self) -> Option<Duration> {
        self.next_non_rendering_delay()
            .into_iter()
            .chain(self.next_rendering_delay())
            .min()
    }

    // Rendering callbacks run in the outer animation phase. A due rendering
    // deadline must wake that phase without keeping executable checkpoints busy.
    fn next_non_rendering_delay(&mut self) -> Option<Duration> {
        if self.has_standalone_script_close() {
            return Some(Duration::ZERO);
        }
        if self
            .active_root_realm_ids()
            .into_iter()
            .filter(|realm| !self.realm_is_parked(*realm))
            .filter_map(|realm| self.binding_states.get(&realm))
            .any(|state| {
                !state.execution_retired()
                    && !state.is_unloading()
                    && state.has_pending_default_actions()
            })
        {
            return Some(Duration::ZERO);
        }
        if self.has_background_script_reports() {
            return Some(Duration::ZERO);
        }
        if self
            .binding_states
            .iter()
            .filter(|(realm, _)| !self.realm_is_parked(**realm))
            .map(|(_, state)| state)
            .filter(|state| !state.is_unloading())
            .any(|state| {
                state.has_history_traversal()
                    || state.has_pending_control_event()
                    || state.has_pending_media_query_change()
                    || state.has_pending_window_message()
                    || state.intersection_observer_ready()
                    || state.performance_observer_ready()
                    || state.stylesheet_event_ready()
                    || state.has_pending_native_csp_event()
                    || state.native_platform_ready()
            })
        {
            return Some(Duration::ZERO);
        }
        let now = Instant::now();
        let native = self
            .binding_states
            .iter()
            .filter(|(realm, _)| !self.realm_is_parked(**realm))
            .map(|(_, state)| state)
            .filter(|state| !state.is_unloading())
            .filter_map(|state| oryn_js::v8_bindings::native_input_delay(state, now))
            .min();
        let unloading = self
            .binding_states
            .iter()
            .filter(|(_, state)| state.is_unloading())
            .map(|(realm, state)| (*realm, state.document_generation()))
            .collect::<BTreeSet<_>>();
        let timers = self
            .isolate
            .get_slot_mut::<TimerRegistry>()
            .and_then(|registry| {
                registry
                    .scheduler
                    .advance_to(registry.started.elapsed().as_millis() as u64);
                registry
                    .scheduler
                    .next_timer_delay_ms_if(|task| {
                        !unloading.contains(&(task.realm, task.document_generation))
                    })
                    .map(Duration::from_millis)
            });
        let shared_waits = self
            .binding_states
            .iter()
            .filter(|(realm, _)| !self.realm_is_parked(**realm))
            .map(|(_, state)| state)
            .filter(|state| !state.is_unloading())
            .filter_map(|state| state.native_shared_wait_delay())
            .min();
        native
            .into_iter()
            .chain(timers)
            .chain(self.module_report_delay())
            .chain(self.css_dependency_delay())
            .chain(self.dynamic_import_completion_delay())
            .chain(self.module_graph_retry_delay())
            .chain(self.module_preload_delay())
            .chain(self.external_template_delay())
            .chain(self.general_preload_delay())
            .chain(self.module_settlement_delay())
            .chain(shared_waits)
            .chain(
                self.binding_states
                    .iter()
                    .filter(|(realm, _)| !self.realm_is_parked(**realm))
                    .map(|(_, state)| state)
                    .filter(|state| !state.is_unloading())
                    .filter_map(|state| {
                        state
                            .native_platform_delay()
                            .into_iter()
                            .chain(state.stylesheet_event_delay())
                            .chain(state.performance_observer_delay())
                            .chain(state.intersection_observer_delay())
                            .min()
                    }),
            )
            .min()
    }

    pub(super) fn worker_tasks_stopped(&self) -> bool {
        self.isolate
            .get_slot::<AgentExecution>()
            .is_some_and(|agent| agent.closing.load(Ordering::Acquire))
    }
    pub fn drain_scheduled_timers(&mut self, wait: bool) -> Result<usize, HostError> {
        self.drain_scheduled_timers_with_terminal_control(wait, None)
    }

    pub(crate) fn drain_scheduled_timers_with_terminal_control(
        &mut self,
        wait: bool,
        terminal: Option<&crate::page_terminal_control::PageTerminalControl>,
    ) -> Result<usize, HostError> {
        self.drain_native_tasks(wait, terminal, true)
    }

    fn drain_native_tasks(
        &mut self,
        wait: bool,
        terminal: Option<&crate::page_terminal_control::PageTerminalControl>,
        admit_timers: bool,
    ) -> Result<usize, HostError> {
        let _stage = crate::native_timing::stage("host.timers");
        let retired = || terminal.is_some_and(|control| control.retired());
        if self.worker_tasks_stopped() || retired() {
            return Ok(0);
        }
        let closed = {
            let _stage = crate::native_timing::stage("host.timers.script_closes");
            self.poll_standalone_script_closes()?
        };
        if closed != 0 {
            return Ok(closed);
        }
        {
            let _stage = crate::native_timing::stage("host.timers.background_reports");
            self.drain_background_script_reports()?
        };
        #[cfg(unix)]
        let external_delivered = {
            let _stage = crate::native_timing::stage("host.timers.remote_messages");
            self.poll_remote_message_tasks()?
        };
        #[cfg(not(unix))]
        let external_delivered = 0;
        if admit_timers && wait
            && let Some(delay) = self.next_timer_delay()
            && !delay.is_zero()
            && delay <= self.execution_wall_limit
        {
            std::thread::sleep(delay);
        }
        let now_ms = self
            .isolate
            .get_slot::<TimerRegistry>()
            .map(|registry| registry.started.elapsed().as_millis() as u64)
            .unwrap_or(0);
        if let Some(registry) = self.isolate.get_slot_mut::<TimerRegistry>() {
            registry.scheduler.advance_to(now_ms);
        }
        let mut delivered = external_delivered;
        // Each page owner pumps its own root and live child frames. Shared
        // standalone roots have independent PageState drivers; polling them
        // here repeats their platform sweep in every sibling's actor turn.
        // Direct standalone host drains retain global scheduling.
        let page_realms = terminal.map(|_| self.active_root_realm_ids());
        let input_realms = self
            .binding_states
            .iter()
            .filter(|(realm, _)| page_realms.as_ref().is_none_or(|realms| realms.contains(realm)))
            .filter(|(realm, _)| !self.realm_is_parked(**realm))
            .filter(|(_, state)| !state.is_unloading())
            .map(|(realm, state)| (*realm, state.clone()))
            .collect::<Vec<_>>();
        for (realm, state) in input_realms {
            let _phase = crate::ipc_progress::close_admission::phase("timer_input_defaults");
            if retired() {
                return Ok(delivered);
            }
            let context = self
                .contexts
                .get(&realm)
                .cloned()
                .ok_or(HostError::CachedAuthorityExpired)?;
            v8::scope!(let handles, &mut self.isolate);
            let context = v8::Local::new(handles, &context);
            let scope = &mut v8::ContextScope::new(handles, context);
            delivered +=
                oryn_js::v8_bindings::poll_native_input_defaults(scope, &state, Instant::now())
                    .map_err(|error| HostError::Execution(error.to_string()))?;
        }
        let selection_realms = self
            .binding_states
            .iter()
            .filter(|(realm, _)| page_realms.as_ref().is_none_or(|realms| realms.contains(realm)))
            .filter(|(realm, state)| !self.realm_is_parked(**realm) && !state.is_unloading())
            .map(|(realm, state)| (*realm, state.clone()))
            .collect::<Vec<_>>();
        for (realm, state) in selection_realms {
            let _phase = crate::ipc_progress::close_admission::phase("timer_platform_defaults");
            if retired() {
                return Ok(delivered);
            }
            let Some(context) = self.contexts.get(&realm).cloned() else {
                continue;
            };
            v8::scope!(let handles,&mut self.isolate);
            let context = v8::Local::new(handles, &context);
            let scope = &mut v8::ContextScope::new(handles, context);
            delivered += {
                let _stage = crate::native_timing::stage("host.timers.selection");
                oryn_js::v8_bindings::poll_native_selection_defaults(scope, &state, Instant::now())
                    .map_err(|error| HostError::Execution(error.to_string()))?
            };
            delivered += {
                let _stage = crate::native_timing::stage("host.timers.media_queries");
                oryn_js::v8_bindings::poll_native_media_queries(scope, &state)
                    .map_err(|error| HostError::Execution(error.to_string()))?
            };
            delivered += {
                let _stage = crate::native_timing::stage("host.timers.visual_viewport");
                oryn_js::v8_bindings::poll_native_visual_viewport(scope, &state)
                    .map_err(|error| HostError::Execution(error.to_string()))?
            };
            delivered += {
                let _stage = crate::native_timing::stage("host.timers.platform");
                oryn_js::v8_bindings::poll_native_platform(scope, &state)
                    .map_err(|error| HostError::Execution(error.to_string()))?
            };
        }
        let batch_started = Instant::now();
        let batch_delivered = delivered;
        while delivered < 1024 {
            let _phase = crate::ipc_progress::close_admission::phase("timer_task_admission");
            // Yield only between complete tasks/microtask checkpoints. An
            // actor turn must not drain hundreds of callbacks ahead of Close.
            if self.worker_tasks_stopped()
                || retired()
                || (terminal.is_some()
                    && !wait
                    && delivered > batch_delivered
                    && batch_started.elapsed() >= Duration::from_millis(8))
            {
                break;
            }
            let control_state = self
                .binding_states
                .iter()
                .find(|(realm, state)| {
                    if self.realm_is_parked(**realm) || state.is_unloading() {
                        return false;
                    }
                    state.has_pending_control_event()
                        || state.has_pending_window_message()
                        || state.intersection_observer_ready()
                        || state.performance_observer_ready()
                        || state.stylesheet_event_ready()
                        || state.has_pending_native_csp_event()
                })
                .map(|(realm, state)| (*realm, state.clone()));
            if let Some((realm, state)) = control_state {
                let _phase = crate::ipc_progress::close_admission::phase("timer_control_event");
                let Some(context) = self.contexts.get(&realm).cloned() else {
                    break;
                };
                let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
                v8::scope!(let handle_scope, &mut self.isolate);
                let context = v8::Local::new(handle_scope, &context);
                let scope = &mut v8::ContextScope::new(handle_scope, context);
                if state.has_pending_control_event() {
                    deliver_control_event(scope, &state)
                } else if state.has_pending_window_message() {
                    oryn_js::v8_bindings::window_messages::deliver(scope, &state)
                } else if state.intersection_observer_ready() {
                    oryn_js::v8_bindings::deliver_intersection_observer(scope, &state)
                } else if state.performance_observer_ready() {
                    oryn_js::v8_bindings::deliver_performance_observer(scope, &state)
                } else if state.has_pending_native_csp_event() {
                    oryn_js::v8_bindings::deliver_native_csp_event(scope, &state)
                } else {
                    oryn_js::v8_bindings::deliver_stylesheet_event(scope, &state)
                }
                .map_err(|error| HostError::Execution(error.to_string()))?;
                queue_mutation_observer_microtask(scope);
                scope.perform_microtask_checkpoint();
                if deadline.expired() || scope.is_execution_terminating() {
                    scope.cancel_terminate_execution();
                    reset_mutation_observer_microtask_after_termination(scope);
                    return Err(HostError::LimitExceeded {
                        resource: "control event task wall time".into(),
                    });
                }
                delivered += 1;
                continue;
            }
            // Empty completion probes may publish native document events,
            // but do not authorize a peer timer task. Explicit page/timer
            // turns and real completion fairness boundaries admit timers.
            if !admit_timers {
                break;
            }
            let unloading = self
                .binding_states
                .iter()
                .filter(|(_, state)| state.is_unloading())
                .map(|(realm, state)| (*realm, state.document_generation()))
                .collect::<BTreeSet<_>>();
            let mut timer_id = None;
            if let Some(registry) = self.isolate.get_slot_mut::<TimerRegistry>() {
                registry.scheduler.run_checkpoint_if(
                    |task| !unloading.contains(&(task.realm, task.document_generation)),
                    |task| timer_id = Some(task.payload),
                );
            }
            let Some(timer_id) = timer_id else { break };
            let timer = self
                .isolate
                .get_slot::<TimerRegistry>()
                .and_then(|registry| registry.callbacks.get(&timer_id))
                .map(|timer| {
                    (
                        timer.context.clone(),
                        timer.callback.clone(),
                        timer.arguments.clone(),
                        timer.repeating,
                        timer.nesting_level,
                    )
                });
            let Some((context, callback, arguments, repeating, nesting_level)) = timer else {
                continue;
            };
            if !repeating && let Some(registry) = self.isolate.get_slot_mut::<TimerRegistry>() {
                registry.callbacks.remove(&timer_id);
            }
            {
                let _phase = crate::ipc_progress::close_admission::phase("timer_callback");
                let nesting_state = self
                    .isolate
                    .get_slot::<TimerRegistry>()
                    .expect("timer registry")
                    .task_nesting
                    .clone();
                let nesting = TimerTaskNesting::enter(nesting_state, nesting_level);
                let _deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
                v8::scope!(let handle_scope, &mut self.isolate);
                let context = v8::Local::new(handle_scope, &context);
                let scope = &mut v8::ContextScope::new(handle_scope, context);
                queue_mutation_observer_microtask(scope);
                v8::tc_scope!(let scope, scope);
                let callback = v8::Local::new(scope, &callback);
                let arguments = arguments
                    .iter()
                    .map(|value| v8::Local::new(scope, value))
                    .collect::<Vec<_>>();
                let receiver = context.global(scope).into();
                if callback.call(scope, receiver, &arguments).is_none() {
                    if _deadline.expired() || scope.is_execution_terminating() {
                        scope.cancel_terminate_execution();
                        reset_mutation_observer_microtask_after_termination(scope);
                        if let Some(registry) = scope.get_slot_mut::<TimerRegistry>()
                            && let Some(timer) = registry.callbacks.remove(&timer_id)
                        {
                            registry.scheduler.cancel_timer(timer.scheduled_id);
                        }
                        return Err(HostError::LimitExceeded {
                            resource: "timer callback wall time".into(),
                        });
                    }
                    let exception = scope.exception();
                    let message = scope.message();
                    scope.reset();
                    // The callback's realm, not the timer-owning Window
                    // or the thrown object's realm, receives ErrorEvent.
                    // Ordinary script errors do not terminate the event loop.
                    if let Some(exception) = exception {
                        report_timer_exception(scope, callback, exception, message)?;
                    }
                }
                // Microtasks are not timer tasks and must not inherit the
                // callback's nesting level. RAII also restores it on errors.
                drop(nesting);
                queue_mutation_observer_microtask(scope);
                scope.perform_microtask_checkpoint();
                if _deadline.expired() || scope.is_execution_terminating() {
                    scope.cancel_terminate_execution();
                    reset_mutation_observer_microtask_after_termination(scope);
                    if let Some(registry) = scope.get_slot_mut::<TimerRegistry>()
                        && let Some(timer) = registry.callbacks.remove(&timer_id)
                    {
                        registry.scheduler.cancel_timer(timer.scheduled_id);
                    }
                    return Err(HostError::LimitExceeded {
                        resource: "wall time".into(),
                    });
                }
            }
            if repeating && let Some(registry) = self.isolate.get_slot_mut::<TimerRegistry>() {
                registry
                    .scheduler
                    .advance_to(registry.started.elapsed().as_millis() as u64);
                if let Some(timer) = registry.callbacks.get_mut(&timer_id) {
                    let environment = timer.environment;
                    timer.scheduled_id = registry.scheduler.set_timeout_with(
                        nested_timer_delay(timer.delay_ms, timer.nesting_level),
                        |_| Task {
                            source: TaskSource::Timer,
                            realm: environment.realm,
                            document_generation: environment.document_generation,
                            action_id: None,
                            payload: timer_id,
                        },
                    );
                    timer.nesting_level = timer.nesting_level.saturating_add(1);
                }
            }
            if !repeating && let Some(registry) = self.isolate.get_slot_mut::<TimerRegistry>() {
                registry.callbacks.remove(&timer_id);
            }
            delivered += 1;
        }
        crate::native_timing::count("host.timers.delivered", delivered as u64);
        Ok(delivered)
    }

    /// Deliver parent-broker completions outside the originating V8
    /// callback. Waiting is bounded by the page execution wall limit and
    /// all ready completions run a real V8 microtask checkpoint.
    pub fn drain_async_fetches(&mut self, wait: bool) -> Result<usize, HostError> {
        self.drain_async_fetches_with_terminal_control(wait, None)
    }

    pub(crate) fn drain_async_fetches_with_terminal_control(
        &mut self,
        wait: bool,
        terminal: Option<&crate::page_terminal_control::PageTerminalControl>,
    ) -> Result<usize, HostError> {
        let _stage = crate::native_timing::stage("host.async_fetch");
        if self.worker_tasks_stopped() || terminal.is_some_and(|control| control.retired()) {
            return Ok(0);
        }
        let closed = self.poll_standalone_script_closes()?;
        if closed != 0 {
            return Ok(closed);
        }
        let imports_delivered = self.poll_external_templates()?
            + self.poll_general_preloads()?
            + self.poll_module_preloads()?
            + self.poll_dynamic_imports()?
            + self.poll_module_reports()?
            + self.poll_module_settlements()?;

        let deadline = std::time::Instant::now() + self.execution_wall_limit;
        let mut delivered = imports_delivered;
        loop {
            if self.worker_tasks_stopped() {
                break;
            }
            #[cfg(unix)]
            let parked = self
                .parked_documents
                .values()
                .flat_map(|document| {
                    document
                        .suspended_environments
                        .iter()
                        .map(|environment| environment.realm)
                })
                .collect::<BTreeSet<_>>();
            #[cfg(not(unix))]
            let parked = BTreeSet::<RealmId>::new();
            let completion = {
                let fetches = self
                    .isolate
                    .get_slot_mut::<AsyncFetches>()
                    .expect("asynchronous fetch registry installed");
                let ready = fetches.deferred.keys().copied().find(|id| {
                    fetches
                        .pending
                        .get(id)
                        .is_none_or(|pending| !parked.contains(&pending.environment.realm))
                });
                if let Some(id) = ready {
                    fetches.deferred.remove(&id)
                } else if !fetches
                    .pending
                    .values()
                    .any(|pending| !parked.contains(&pending.environment.realm))
                {
                    fetches.receiver.try_recv().ok()
                } else if wait {
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    if remaining.is_zero() {
                        return Err(HostError::LimitExceeded {
                            resource: "asynchronous fetch wall time".into(),
                        });
                    }
                    match fetches
                        .receiver
                        .recv_timeout(remaining.min(Duration::from_millis(50)))
                    {
                        Ok(completion) => Some(completion),
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            return Err(HostError::Execution(
                                "asynchronous fetch completion channel closed".into(),
                            ));
                        }
                    }
                } else {
                    match fetches.receiver.try_recv() {
                        Ok(completion) => Some(completion),
                        Err(mpsc::TryRecvError::Empty) => None,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            return Err(HostError::Execution(
                                "asynchronous fetch completion channel closed".into(),
                            ));
                        }
                    }
                }
            };
            let Some(mut completion) = completion else {
                break;
            };
            if self
                .isolate
                .get_slot_mut::<AsyncFetches>()
                .unwrap()
                .retired
                .remove(&completion.operation_id)
            {
                continue;
            }
            {
                let fetches = self.isolate.get_slot_mut::<AsyncFetches>().unwrap();
                if fetches
                    .pending
                    .get(&completion.operation_id)
                    .is_some_and(|pending| parked.contains(&pending.environment.realm))
                {
                    if fetches.deferred.contains_key(&completion.operation_id) {
                        return Err(HostError::Execution(
                            "duplicate suspended fetch completion".into(),
                        ));
                    }
                    fetches.deferred.insert(completion.operation_id, completion);
                    continue;
                }
            }
            let pending = self
                .isolate
                .get_slot_mut::<AsyncFetches>()
                .expect("asynchronous fetch registry installed")
                .pending
                .remove(&completion.operation_id)
                .ok_or_else(|| {
                    HostError::Execution(format!(
                        "unmatched asynchronous fetch completion {}",
                        completion.operation_id
                    ))
                })?;
            #[cfg(unix)]
            if let Some((stream, success)) = completion.stream_terminal {
                let fetches = self
                    .isolate
                    .get_slot_mut::<AsyncFetches>()
                    .expect("native fetch registry");
                fetches.finish_stream(pending.environment, stream, success);
            }
            let _execution_deadline = ExecutionDeadline::arm(
                &self.isolate,
                deadline.saturating_duration_since(Instant::now()),
            );
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &pending.context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            queue_mutation_observer_microtask(scope);
            let resolver = v8::Local::new(scope, &pending.resolver);
            let text = match &completion.result {
                Ok(value) | Err(value) => value,
            };
            #[cfg(test)]
            let injected_failure = FAIL_FETCH_DELIVERY_ONCE.with(|fail| fail.replace(false));
            #[cfg(not(test))]
            let injected_failure = false;
            let value = if injected_failure {
                None
            } else {
                v8::String::new(scope, text)
            };
            let Some(value) = value else {
                // No resolver has been entered. Keep both the original
                // promise and its owned response for the next checkpoint.
                let fetches = scope
                    .get_slot_mut::<AsyncFetches>()
                    .expect("native fetch registry");
                fetches.pending.insert(completion.operation_id, pending);
                fetches.deferred.insert(completion.operation_id, completion);
                return Err(HostError::Execution(
                    "fetch delivery argument allocation failed".into(),
                ));
            };
            match &completion.result {
                Ok(_) => {
                    let accepted = resolver.resolve(scope, value.into()) == Some(true);
                    #[cfg(unix)]
                    if accepted && let Some(network) = completion.network.take() {
                        if let Some(stream) = network.stream_id() {
                            let fetches = scope
                                .get_slot_mut::<AsyncFetches>()
                                .expect("native fetch registry");
                            if let Some((_, old)) = fetches
                                .streams
                                .insert(stream, (pending.environment, network))
                            {
                                old.cancel();
                            }
                        } else {
                            network.finish();
                        }
                    }
                    #[cfg(not(unix))]
                    let _ = accepted;
                }
                Err(_) => {
                    let exception = v8::Exception::error(scope, value);
                    resolver.reject(scope, exception);
                }
            }
            queue_mutation_observer_microtask(scope);
            scope.perform_microtask_checkpoint();
            if _execution_deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            delivered += 1;
        }
        delivered += self.drain_generated_platform_operations(wait, terminal)?;
        crate::native_timing::count("host.async_fetch.delivered", delivered as u64);
        Ok(delivered)
    }

    fn drain_generated_platform_operations(
        &mut self,
        wait: bool,
        terminal: Option<&crate::page_terminal_control::PageTerminalControl>,
    ) -> Result<usize, HostError> {
        // A transaction keep-alive loop is legitimate browser activity:
        // every success handler may enqueue the next request forever.
        // Bound one scheduler checkpoint so that such a chain yields to
        // other page tasks instead of being misreported as a wall-time
        // violation. Operations that make no progress still wait until
        // the normal execution deadline and fail closed.
        const MAX_COMPLETIONS_PER_CHECKPOINT: usize = 1_024;
        let deadline = Instant::now() + self.execution_wall_limit;
        let mut delivered = 0;
        loop {
            if terminal.is_some_and(|control| control.retired()) {
                break;
            }
            // Independent roots have their own PageState drivers. A page-owned
            // fixed point pumps only its root and live descendants; direct host
            // drains without an owner retain the global standalone semantics.
            let page_realms = terminal.map(|_| self.active_root_realm_ids());
            let mut pass = 0;
            let scheduled = self
                .binding_states
                .iter()
                .filter(|(realm, _)| page_realms.as_ref().is_none_or(|realms| realms.contains(realm)))
                .filter_map(|(realm, state)| {
                    if state.execution_retired() || self.realm_is_parked(*realm) {
                        return None;
                    }
                    #[cfg(test)]
                    let _probe = crate::ipc_progress::close_admission::phase(
                        if *realm == self.active_realm {
                            "generated_platform_selected_probe"
                        } else {
                            "generated_platform_sibling_probe"
                        },
                    );
                    self.contexts
                        .get(realm)
                        .cloned()
                        .map(|context| (context, state.clone()))
                })
                .collect::<Vec<_>>();
            for (context, state) in scheduled {
                if terminal.is_some_and(|control| control.retired()) {
                    return Ok(delivered + pass);
                }
                // Earlier completion callbacks may synchronously retire another
                // scheduled owner. Never resume its discarded execution queues.
                if state.execution_retired() {
                    continue;
                }
                let _execution_deadline = ExecutionDeadline::arm(
                    &self.isolate,
                    deadline.saturating_duration_since(Instant::now()),
                );
                v8::scope!(let handle_scope, &mut self.isolate);
                let context = v8::Local::new(handle_scope, &context);
                let scope = &mut v8::ContextScope::new(handle_scope, context);
                queue_mutation_observer_microtask(scope);
                // Each platform completion is a task; its promise jobs
                // must run before the next completion is delivered.
                let completed = state
                    .drain_platform_completions(scope, 1)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                pass += completed;
                // A preceding realm's task does not authorize an additional
                // post-task checkpoint for this realm's empty completion probe.
                if completed > 0 {
                    #[cfg(test)]
                    let _phase = crate::ipc_progress::close_admission::phase(
                        "platform_completion_microtask",
                    );
                    queue_mutation_observer_microtask(scope);
                    scope.perform_microtask_checkpoint();
                    if _execution_deadline.expired() || scope.is_execution_terminating() {
                        scope.cancel_terminate_execution();
                        reset_mutation_observer_microtask_after_termination(scope);
                        return Err(HostError::LimitExceeded {
                            resource: "wall time".into(),
                        });
                    }
                    state
                        .finish_platform_task_checkpoint()
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                }
            }
            delivered += pass;
            // Timers are peer page tasks. Run ready timers between
            // platform-completion turns so a self-sustaining request
            // chain cannot starve cancellation, transaction release, or
            // other zero-delay work.
            // An empty nonblocking completion probe is not a platform task
            // turn. Leave peer timers with the page scheduler instead of
            // polling every live binding again at each fetch checkpoint.
            // Waiting probes still service timers, and every real completion
            // retains the fairness boundary above. Direct standalone host
            // checkpoints have no independent PageState task driver: they
            // must also pump control/message/stylesheet/CSP tasks when the
            // platform completion queue is empty.
            if pass != 0 || wait {
                delivered += self.drain_scheduled_timers_with_terminal_control(false, terminal)?;
            } else if terminal.is_none() {
                delivered += self.drain_native_tasks(false, None, false)?;
            }
            if delivered >= MAX_COMPLETIONS_PER_CHECKPOINT {
                break;
            }
            let pending = self
                .binding_states
                .iter()
                .filter(|(realm, _)| page_realms.as_ref().is_none_or(|realms| realms.contains(realm)))
                .filter(|(realm, _)| !self.realm_is_parked(**realm))
                .map(|(_, state)| state)
                .any(|state| state.has_pending_non_image_work());
            if !wait || !pending {
                break;
            }
            if Instant::now() >= deadline {
                return Err(HostError::LimitExceeded {
                    resource: "generated platform operation wall time".into(),
                });
            }
            if pass == 0 {
                // Parent-service database operations are normally tiny.
                // A millisecond polling floor compounds into seconds for
                // large cursor/partition suites, even though the ordered
                // service has already made the next completion runnable.
                // Yield briefly without busy-spinning or delaying network
                // operations by an entire scheduler quantum.
                std::thread::sleep(Duration::from_micros(50));
            }
        }
        Ok(delivered)
    }

    /// Native admission sequence and active fetch custody. The sequence
    /// also detects a request admitted and completed in one checkpoint.
    #[cfg(test)]
    pub(crate) fn async_fetch_activity(&self) -> (u64, bool) {
        (
            self.isolate
                .get_slot::<AsyncFetches>()
                .map_or(0, |fetches| fetches.next_operation_id),
            self.has_pending_async_fetches(),
        )
    }

    pub(crate) fn async_fetch_activity_for_root(&self, realm: RealmId) -> (u64, bool) {
        let Some(root) = self
            .isolate
            .get_slot::<RealmEndpointRegistry>()
            .and_then(|registry| registry.0.get(&realm))
            .map(|(environment, _)| *environment)
        else {
            return (0, false);
        };
        let environments = self
            .document_environments(root)
            .into_iter()
            .filter(|environment| !self.realm_is_parked(environment.realm))
            .map(|environment| environment.id)
            .collect::<BTreeSet<_>>();
        self.isolate
            .get_slot::<AsyncFetches>()
            .map_or((0, false), |fetches| {
                let admitted = environments
                    .iter()
                    .filter_map(|id| fetches.last_admission.get(id))
                    .copied()
                    .max()
                    .unwrap_or(0);
                let pending = fetches
                    .pending
                    .values()
                    .any(|pending| environments.contains(&pending.environment.id));
                (admitted, pending)
            })
    }

    pub fn has_pending_async_fetches(&self) -> bool {
        self.isolate
            .get_slot::<AsyncFetches>()
            .is_some_and(|fetches| {
                fetches
                    .pending
                    .values()
                    .any(|pending| !self.realm_is_parked(pending.environment.realm))
            })
    }

    pub(crate) fn has_pending_root_navigation(&self) -> bool {
        self.active_root_realm_ids().into_iter().any(|realm| {
            !self.realm_is_parked(realm)
                && self.binding_states.get(&realm).is_some_and(|state| {
                    !state.execution_retired()
                        && (state.has_location_navigation() || state.has_history_traversal())
                })
        })
    }

    pub fn has_pending_page_work(&mut self) -> bool {
        if self.has_standalone_script_close() {
            return true;
        }
        if self.has_pending_root_navigation() {
            return true;
        }
        if self.has_pending_dynamic_imports(false) {
            return true;
        }
        #[cfg(unix)]
        if self.has_css_dependencies() {
            return true;
        }
        self.has_pending_async_fetches()
            || self.next_timer_delay().is_some()
            || self
                .binding_states
                .iter()
                .filter(|(realm, _)| !self.realm_is_parked(**realm))
                .map(|(_, state)| state)
                .any(|state| {
                    state.has_pending_platform_work()
                        || state.has_pending_background_clients()
                        || state.has_closed_document_stream()
                })
    }

    pub fn has_runnable_page_work(&mut self) -> bool {
        self.has_runnable_completion_work(false)
            || self
                .next_non_rendering_delay()
                .is_some_and(|delay| delay.is_zero())
            || self
                .next_rendering_delay()
                .is_some_and(|delay| delay.is_zero())
    }

    pub(crate) fn has_runnable_checkpoint_work(&mut self) -> bool {
        self.has_runnable_completion_work(true)
    }

    fn has_runnable_completion_work(&mut self, root_only: bool) -> bool {
        if self.has_pending_root_navigation() {
            return true;
        }
        if self.has_pending_dynamic_imports(true) {
            return true;
        }
        #[cfg(unix)]
        if self.has_css_dependencies() {
            return true;
        }
        // A document fixed point follows its selected root and child frames.
        // The outer page task scheduler still observes all agent completions;
        // sibling work is retained, not mistaken for this document's mutation.
        let roots = root_only.then(|| self.active_root_realm_ids());
        // External completions are collected by the bounded actor poll. Waiting
        // for one must not force zero-delay polling or repeated checkpoints.
        self.isolate
            .get_slot::<AsyncFetches>()
            .is_some_and(|fetches| {
                fetches.deferred.keys().any(|id| {
                    fetches
                        .pending
                        .get(id)
                        .is_none_or(|pending| {
                            !self.realm_is_parked(pending.environment.realm)
                                && roots.as_ref().is_none_or(|realms| {
                                    realms.contains(&pending.environment.realm)
                                })
                        })
                })
            })
            // Due timers are peer page tasks, not a reason to repeat an
            // unchanged document checkpoint. The outer page scheduler still
            // observes them through has_runnable_page_work above.
            || self
                .binding_states
                .iter()
                .filter(|(realm, _)| !self.realm_is_parked(**realm))
                .filter(|(realm, _)| roots.as_ref().is_none_or(|realms| realms.contains(realm)))
                .map(|(_, state)| state)
                .any(|state| state.has_ready_platform_work() || state.has_closed_document_stream())
    }

    /// Initial native Windows need no compatibility bootstrap to receive
    /// native tasks. Only a completed native bootstrap owns a legacy poll.
    pub(crate) fn poll_runtime_context(&mut self) -> Result<(), HostError> {
        let _stage = crate::native_timing::stage("host.runtime_context");
        if !self.realm_execution_live(self.active_realm) {
            return Ok(());
        }
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("runtime context unavailable".into()))?;
        let bootstrap = {
            v8::scope!(let scope,&mut self.isolate);
            v8::Local::new(scope, &context).get_slot::<RuntimeBootstrapped>()
        };
        let Some(bootstrap) = bootstrap else {
            return Ok(());
        };
        match bootstrap.phase.get() {
            RuntimeBootstrapPhase::Initializing => return Ok(()),
            RuntimeBootstrapPhase::Failed => {
                return Err(HostError::Execution(
                    "runtime bootstrap previously failed".into(),
                ));
            }
            RuntimeBootstrapPhase::Ready => {}
        }
        self.ensure_standalone_author_task()?;
        self.publish_native_resource_timings();
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let poll = {
            let retained = bootstrap.poll.borrow();
            retained
                .as_ref()
                .and_then(|poll| poll.to_local(scope))
                .ok_or_else(|| HostError::Execution("runtime poll custody missing".into()))?
        };
        queue_mutation_observer_microtask(scope);
        let receiver = context.global(scope).into();
        if poll.call(scope, receiver, &[]).is_none() {
            if deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "runtime context poll wall time".into(),
                });
            }
            let detail = scope
                .exception()
                .and_then(|value| value.to_string(scope))
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "runtime context poll failed".into());
            return Err(HostError::Execution(detail));
        }
        queue_mutation_observer_microtask(scope);
        scope.perform_microtask_checkpoint();
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            return Err(HostError::LimitExceeded {
                resource: "runtime context poll wall time".into(),
            });
        }
        if scope.has_caught() {
            let detail = scope
                .exception()
                .and_then(|value| value.to_string(scope))
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "runtime context poll microtask failed".into());
            return Err(HostError::Execution(detail));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::JavaScriptHost;
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    fn document_host() -> RawV8Host {
        let mut host = RawV8Host::new();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        host.install_dom(dom, 1).unwrap();
        host
    }

    #[test]
    fn idle_timer_poll_skips_layout_but_interest_focus_refreshes_it() {
        struct LayoutCalls(std::cell::Cell<usize>);
        impl oryn_js::v8_bindings::BindingLayoutProvider for LayoutCalls {
            fn compute_layout(
                &self,
                _: &std::collections::BTreeMap<oryn_dom::NodeId, BTreeSet<String>>,
            ) -> Result<oryn_js::v8_bindings::BindingLayout, String> {
                self.0.set(self.0.get() + 1);
                Ok(Default::default())
            }
        }
        let mut host = document_host();
        host.execute("document.body.innerHTML='<button id=source interestfor=target>show</button><div id=target>target</div>'").unwrap();
        let provider = Rc::new(LayoutCalls(std::cell::Cell::new(0)));
        host.install_layout_provider(provider.clone()).unwrap();
        provider.0.set(0);
        for _ in 0..3 {
            host.drain_scheduled_timers(false).unwrap();
        }
        assert_eq!(provider.0.get(), 0, "idle polling must not ask for layout");
        let dom = host.binding_states[&host.active_realm].dom();
        let source = dom.borrow().find_by_id("source").unwrap();
        dom.borrow_mut().focus(Some(source)).unwrap();
        host.drain_scheduled_timers(false).unwrap();
        assert!(
            provider.0.get() > 0,
            "interest focus still samples computed delays"
        );
    }

    #[test]
    fn checkpoint_witness_covers_child_mutations_and_refuses_missing_or_borrowed_state() {
        let mut host = document_host();
        host.execute(
            "globalThis.frame=document.createElement('iframe');document.body.append(frame)",
        )
        .unwrap();
        host.adopt_created_realms();
        let before = host.executable_checkpoint_witness().unwrap();
        assert!(before.len() >= 2);
        assert_eq!(host.executable_checkpoint_witness(), Some(before.clone()));
        host.execute("frame.contentDocument.body.textContent='child mutation'")
            .unwrap();
        let after = host.executable_checkpoint_witness().unwrap();
        assert_ne!(before, after);
        let child = after
            .iter()
            .map(|(realm, _)| *realm)
            .find(|realm| *realm != host.active_realm)
            .unwrap();
        let dom = host.binding_states[&child].dom();
        let borrowed = dom.borrow_mut();
        assert!(host.executable_checkpoint_witness().is_none());
        drop(borrowed);
        assert_eq!(host.executable_checkpoint_witness(), Some(after.clone()));
        let context = host.contexts.remove(&child).unwrap();
        assert!(host.executable_checkpoint_witness().is_none());
        assert!(host.has_pending_executable_checkpoint_work(host.active_realm));
        host.contexts.insert(child, context);
        assert_eq!(host.executable_checkpoint_witness(), Some(after));
    }

    #[test]
    fn checkpoint_witness_does_not_consume_pending_realm_admission() {
        let mut host = document_host();
        let context = host.contexts[&host.active_realm].clone();
        let pending = RealmId(u32::MAX);
        host.isolate
            .get_slot_mut::<NativeRealmIdentities>()
            .unwrap()
            .pending_contexts
            .insert(pending, context);
        assert!(host.executable_checkpoint_witness().is_none());
        assert!(host.has_pending_executable_checkpoint_work(host.active_realm));
        assert!(
            host.isolate
                .get_slot::<NativeRealmIdentities>()
                .unwrap()
                .pending_contexts
                .contains_key(&pending)
        );
        host.isolate
            .get_slot_mut::<NativeRealmIdentities>()
            .unwrap()
            .pending_contexts
            .remove(&pending);
        assert!(host.executable_checkpoint_witness().is_some());
    }

    #[test]
    fn checkpoint_witness_refuses_undrained_owned_frame_discard() {
        let mut host = document_host();
        host.execute(
            "globalThis.frame=document.createElement('iframe');document.body.append(frame)",
        )
        .unwrap();
        host.adopt_created_realms();
        let before = host.executable_checkpoint_witness().unwrap();
        assert!(before.len() >= 2);
        host.execute("frame.remove()").unwrap();
        assert!(host.executable_checkpoint_witness().is_none());
        assert!(host.has_pending_executable_checkpoint_work(host.active_realm));
        let frames = host.isolate.get_slot::<FrameLifetimes>().unwrap();
        assert!(!frames.discarded_realms.is_empty());
        assert!(
            frames
                .discarded_owners
                .values()
                .any(|owner| *owner == host.active_realm)
        );
    }

    #[test]
    fn checkpoint_pending_includes_queued_global_observer_without_consuming_it() {
        let mut host = document_host();
        host.execute("globalThis.observed=0;globalThis.observer=new MutationObserver(()=>observed++);observer.observe(document.body,{attributes:true})")
            .unwrap();
        let state = host.binding_states[&host.active_realm].clone();
        let dom = state.dom();
        let body = dom
            .borrow()
            .query_selector(state.document_root(), "body")
            .unwrap()
            .unwrap();
        dom.borrow_mut()
            .set_attribute(body, "data-observer", "pending")
            .unwrap();
        {
            let context = host.contexts[&host.active_realm].clone();
            v8::scope!(let handles, &mut host.isolate);
            let context = v8::Local::new(handles, context);
            let scope = &mut v8::ContextScope::new(handles, context);
            queue_mutation_observer_microtask(scope);
        }
        assert!(has_pending_mutation_observer_microtask(&host.isolate));
        assert!(!state.has_pending_executable_checkpoint_work(true));
        let before = host.executable_checkpoint_witness().unwrap();
        assert!(host.has_pending_executable_checkpoint_work(host.active_realm));
        assert_eq!(host.executable_checkpoint_witness(), Some(before));
        assert!(has_pending_mutation_observer_microtask(&host.isolate));
        host.run_microtask_checkpoint().unwrap();
        assert!(!has_pending_mutation_observer_microtask(&host.isolate));
        assert_eq!(host.evaluate_string("String(observed)").unwrap(), "1");
    }

    #[test]
    fn checkpoint_witness_refuses_stopped_tasks_without_document_mutation() {
        let host = document_host();
        let state = host.binding_states[&host.active_realm].clone();
        let identity = state.executable_checkpoint_identity().unwrap();
        let before = host.executable_checkpoint_witness().unwrap();
        let unloading = state.begin_unload();
        assert_eq!(state.executable_checkpoint_identity(), Some(identity));
        assert!(state.execution_tasks_stopped());
        assert!(host.executable_checkpoint_witness().is_none());
        assert!(host.has_pending_executable_checkpoint_work(host.active_realm));
        drop(unloading);
        assert_eq!(host.executable_checkpoint_witness(), Some(before));
    }

    #[test]
    fn checkpoint_witness_tracks_same_dom_document_stream_replacement() {
        let mut host = document_host();
        let before = host.executable_checkpoint_witness().unwrap();
        let dom = host.binding_states[&host.active_realm].dom();
        host.execute("document.open();document.write('<body>replacement');document.close()")
            .unwrap();
        assert!(Rc::ptr_eq(
            &dom,
            &host.binding_states[&host.active_realm].dom()
        ));
        let after = host.executable_checkpoint_witness().unwrap();
        assert_eq!(before[0].0, after[0].0);
        assert_eq!(before[0].1.instance_id, after[0].1.instance_id);
        assert_ne!(
            before[0].1.document_stream_epoch,
            after[0].1.document_stream_epoch
        );
        assert_ne!(before, after);
    }
}
