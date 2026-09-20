// Included in raw_v8. One exact-environment parser checkpoint preserves the
// pre-parser import map while discarding only provisional module preparations.
static PARSER_MODULE_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct ParserModuleBytes(usize);
impl ParserModuleBytes {
    fn reserve(bytes: usize) -> Result<Self, HostError> {
        if PARSER_MODULE_BYTES
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |used| {
                    used.checked_add(bytes)
                        .filter(|total| *total <= 64 * 1024 * 1024)
                },
            )
            .is_err()
        {
            return Err(HostError::LimitExceeded {
                resource: "parser module checkpoint bytes".into(),
            });
        }
        Ok(Self(bytes))
    }
}
impl Drop for ParserModuleBytes {
    fn drop(&mut self) {
        PARSER_MODULE_BYTES.fetch_sub(self.0, std::sync::atomic::Ordering::AcqRel);
    }
}
pub(crate) struct ParserModuleSnapshot {
    refresh: Option<oryn_js::v8_bindings::DocumentRefreshSnapshot>,
    environment: EnvironmentDescriptor,
    restored: std::cell::Cell<bool>,
    map: crate::import_maps::ImportMap,
    fetch: BTreeSet<String>,
    fetch_pending: BTreeSet<String>,
    typed: BTreeSet<(String, crate::ModuleType)>,
    typed_pending: BTreeSet<(String, crate::ModuleType)>,
    policies: BTreeMap<String, bool>,
    referrers: BTreeMap<String, String>,
    cache: BTreeSet<String>,
    modules: std::collections::HashSet<v8::Global<v8::Module>>,
    statuses: std::collections::HashMap<v8::Global<v8::Module>, v8::ModuleStatus>,
    typed_keys: BTreeSet<(String, crate::ModuleType)>,
    typed_bytes: usize,
    errors: BTreeSet<ModuleGraphKey>,
    receipts: BTreeSet<u64>,
    reports: std::collections::HashSet<v8::Global<v8::PromiseResolver>>,
    preloads: Vec<(Option<NodeId>, ModulePreloadKey, Instant)>,
    preload_seen: BTreeMap<NodeId, ModulePreloadKey>,
    preload_headers: BTreeSet<String>,
    preload_pauses: RefCell<Vec<ModulePreloadPause>>,
    local_resource_preloads: crate::local_resource_preload::Snapshot,
    #[cfg(unix)]
    resource_preloads: Option<crate::context_service::ResourcePreloadSnapshot>,
    general_preloads: Vec<(Option<NodeId>, String, Instant)>,
    general_seen: BTreeMap<NodeId, String>,
    general_headers: BTreeSet<String>,
    general_pauses: RefCell<Vec<GeneralPreloadPause>>,
    _bytes: ParserModuleBytes,
}
impl RawV8Host {
    fn capture_parser_module_snapshot(&mut self) -> Result<ParserModuleSnapshot, HostError> {
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("parser module context missing".into()))?;
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        let environment = current_environment(scope).map_err(HostError::Execution)?;
        let id = environment.id;
        let map = module_import_map(scope);
        let mut size = map.borrow().retained_bytes().saturating_add(1024);
        if let Some(cache) = context.get_slot::<RefCell<ModuleFetchResults>>() {
            let cache = cache.borrow();
            size = size.saturating_add(
                cache
                    .0
                    .keys()
                    .chain(cache.2.iter())
                    .map(|key| key.len() + 96)
                    .sum::<usize>(),
            );
        }
        if let Some(cache) = context.get_slot::<RefCell<TypedModuleFetchResults>>() {
            let cache = cache.borrow();
            size = size.saturating_add(
                cache
                    .0
                    .keys()
                    .chain(cache.2.iter())
                    .map(|(key, _)| key.len() + 96)
                    .sum::<usize>(),
            );
        }
        if let Some(cache) = scope.get_slot::<ModuleCache>() {
            size = size.saturating_add(
                cache
                    .0
                    .keys()
                    .filter(|(owner, _)| *owner == id)
                    .map(|(_, key)| key.len() + 128)
                    .sum::<usize>(),
            );
        }
        if let Some(cache) = scope.get_slot::<TypedModules>() {
            size = size.saturating_add(
                cache
                    .entries
                    .keys()
                    .filter(|(owner, _, _)| *owner == id)
                    .map(|(_, key, _)| key.len() + 128)
                    .sum::<usize>(),
            );
        }
        if let Some(values) = context.get_slot::<RefCell<ModuleFetchPolicies>>() {
            size = size.saturating_add(
                values
                    .borrow()
                    .0
                    .keys()
                    .map(|key| key.len() + 96)
                    .sum::<usize>(),
            );
        }
        if let Some(values) = context.get_slot::<RefCell<ModuleReferrerPolicies>>() {
            size = size.saturating_add(
                values
                    .borrow()
                    .0
                    .iter()
                    .map(|(key, value)| key.len() + value.len() + 96)
                    .sum::<usize>(),
            );
        }
        if let Some(queue) = scope.get_slot::<Rc<RefCell<ModulePreloads>>>() {
            let queue = queue.borrow();
            size = size.saturating_add(
                queue
                    .seen
                    .iter()
                    .filter(|((owner, _), _)| *owner == id)
                    .map(|(_, key)| key.retained_bytes().saturating_add(256))
                    .fold(0usize, usize::saturating_add),
            );
            size = size.saturating_add(
                queue
                    .headers
                    .iter()
                    .filter(|(owner, _)| *owner == id)
                    .map(|(_, value)| value.len() + 96)
                    .sum::<usize>(),
            );
            size = size.saturating_add(
                queue
                    .jobs
                    .iter()
                    .filter(|job| job.environment.id == id)
                    .map(|job| job.key.retained_bytes().saturating_add(256))
                    .fold(0usize, usize::saturating_add),
            );
        }
        if let Some(owners) = scope.get_slot::<ModuleOwners>() {
            size = size.saturating_add(
                owners
                    .0
                    .values()
                    .filter(|owner| **owner == id)
                    .count()
                    .saturating_mul(256),
            );
        }
        if let Some(errors) = context.get_slot::<RefCell<ModuleGraphErrors>>() {
            let errors = errors.borrow();
            size = size
                .saturating_add(
                    errors
                        .parsed
                        .keys()
                        .map(|(url, _)| url.len() + 128)
                        .sum::<usize>(),
                )
                .saturating_add(errors.receipts.len().saturating_mul(96));
        }
        if let Some(reports) = scope.get_slot::<ModuleReportQueue>() {
            size = size.saturating_add(
                reports
                    .jobs
                    .iter()
                    .filter(|job| job.environment.id == id)
                    .count()
                    .saturating_mul(128),
            );
        }
        size = size.saturating_add(
            context
                .get_slot::<V8BindingState>()
                .map_or(0, |state| state.refresh_checkpoint_bytes()),
        );
        #[cfg(unix)]
        let resource_preloads = current_context_endpoint(scope)
            .map(|endpoint| endpoint.client.capture_resource_preloads());
        #[cfg(unix)]
        {
            size = size.saturating_add(
                resource_preloads
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.retained_bytes()),
            );
        }
        if let Some(queue) = scope.get_slot::<Rc<RefCell<GeneralPreloads>>>() {
            let queue = queue.borrow();
            size = size.saturating_add(
                queue
                    .headers
                    .iter()
                    .filter(|(owner, _)| *owner == id)
                    .map(|(_, key)| key.len() + 96)
                    .sum::<usize>(),
            );
            size = size
                .saturating_add(
                    queue
                        .seen
                        .iter()
                        .filter(|((owner, _), _)| *owner == id)
                        .map(|(_, key)| key.len() + 128)
                        .sum::<usize>(),
                )
                .saturating_add(
                    queue
                        .jobs
                        .iter()
                        .filter(|job| job.environment.id == id)
                        .map(|job| job.key.len() + 128)
                        .sum::<usize>(),
                );
        }
        let local_resource_preloads = native_script_report_queue(scope).3.snapshot();
        size = size.saturating_add(local_resource_preloads.retained_bytes());
        let bytes = ParserModuleBytes::reserve(size)?;
        let refresh = context
            .get_slot::<V8BindingState>()
            .map(|state| state.capture_refresh_checkpoint());
        let (fetch, fetch_pending) = context
            .get_slot::<RefCell<ModuleFetchResults>>()
            .map(|cache| {
                let cache = cache.borrow();
                (cache.0.keys().cloned().collect(), cache.2.clone())
            })
            .unwrap_or_default();
        let (typed, typed_pending) = context
            .get_slot::<RefCell<TypedModuleFetchResults>>()
            .map(|cache| {
                let cache = cache.borrow();
                (cache.0.keys().cloned().collect(), cache.2.clone())
            })
            .unwrap_or_default();
        let policies = context
            .get_slot::<RefCell<ModuleFetchPolicies>>()
            .map(|values| values.borrow().0.clone())
            .unwrap_or_default();
        let referrers = context
            .get_slot::<RefCell<ModuleReferrerPolicies>>()
            .map(|values| values.borrow().0.clone())
            .unwrap_or_default();
        let cache = scope
            .get_slot::<ModuleCache>()
            .map(|cache| {
                cache
                    .0
                    .keys()
                    .filter(|(owner, _)| *owner == id)
                    .map(|(_, key)| key.clone())
                    .collect()
            })
            .unwrap_or_default();
        let modules: std::collections::HashSet<v8::Global<v8::Module>> = scope
            .get_slot::<ModuleOwners>()
            .map(|owners| {
                owners
                    .0
                    .iter()
                    .filter(|(_, owner)| **owner == id)
                    .map(|(module, _)| module.clone())
                    .collect()
            })
            .unwrap_or_default();
        let statuses = modules
            .iter()
            .map(|module| {
                let local = v8::Local::new(scope, module);
                (module.clone(), local.get_status())
            })
            .collect();
        let (typed_keys, typed_bytes) = scope
            .get_slot::<TypedModules>()
            .map(|typed| {
                (
                    typed
                        .entries
                        .keys()
                        .filter(|(owner, _, _)| *owner == id)
                        .map(|(_, key, kind)| (key.clone(), *kind))
                        .collect(),
                    typed.source_bytes.get(&id).copied().unwrap_or(0),
                )
            })
            .unwrap_or_default();
        let (errors, receipts) = context
            .get_slot::<RefCell<ModuleGraphErrors>>()
            .map(|errors| {
                let errors = errors.borrow();
                (
                    errors.parsed.keys().cloned().collect(),
                    errors.receipts.keys().copied().collect(),
                )
            })
            .unwrap_or_default();
        let reports = scope
            .get_slot::<ModuleReportQueue>()
            .map(|queue| {
                queue
                    .jobs
                    .iter()
                    .filter(|job| job.environment.id == id)
                    .map(|job| job.resolver.clone())
                    .collect()
            })
            .unwrap_or_default();
        let (preloads, preload_seen, preload_headers) = scope
            .get_slot::<Rc<RefCell<ModulePreloads>>>()
            .map(|queue| {
                let queue = queue.borrow();
                (
                    queue
                        .jobs
                        .iter()
                        .filter(|job| job.environment.id == id)
                        .map(|job| {
                            (
                                job.target.as_ref().map(|target| target.current().1),
                                job.key.clone(),
                                job.deadline,
                            )
                        })
                        .collect(),
                    queue
                        .seen
                        .iter()
                        .filter(|((owner, _), _)| *owner == id)
                        .map(|((_, node), key)| (*node, key.clone()))
                        .collect(),
                    queue
                        .headers
                        .iter()
                        .filter(|(owner, _)| *owner == id)
                        .map(|(_, header)| header.clone())
                        .collect(),
                )
            })
            .unwrap_or_default();
        let preload_pauses = RefCell::new(
            scope
                .get_slot::<Rc<RefCell<ModulePreloads>>>()
                .map(|queue| {
                    queue
                        .borrow()
                        .jobs
                        .iter()
                        .filter(|job| job.environment.id == id)
                        .map(ModulePreload::pause_preparation)
                        .collect()
                })
                .unwrap_or_default(),
        );
        let general_headers = scope
            .get_slot::<Rc<RefCell<GeneralPreloads>>>()
            .map(|queue| {
                queue
                    .borrow()
                    .headers
                    .iter()
                    .filter(|(owner, _)| *owner == id)
                    .map(|(_, key)| key.clone())
                    .collect()
            })
            .unwrap_or_default();
        let (general_preloads, general_seen, general_pauses) = scope
            .get_slot::<Rc<RefCell<GeneralPreloads>>>()
            .map(|queue| {
                let queue = queue.borrow();
                (
                    queue
                        .jobs
                        .iter()
                        .filter(|job| job.environment.id == id)
                        .map(|job| {
                            (
                                job.target.as_ref().map(|target| target.current().1),
                                job.key.clone(),
                                job.deadline,
                            )
                        })
                        .collect(),
                    queue
                        .seen
                        .iter()
                        .filter(|((owner, _), _)| *owner == id)
                        .map(|((_, node), key)| (*node, key.clone()))
                        .collect(),
                    RefCell::new(
                        queue
                            .jobs
                            .iter()
                            .filter(|job| job.environment.id == id)
                            .map(GeneralPreload::pause_preparation)
                            .collect(),
                    ),
                )
            })
            .unwrap_or_default();
        let map = map.borrow().clone();
        Ok(ParserModuleSnapshot {
            local_resource_preloads,
            #[cfg(unix)]
            resource_preloads,
            general_preloads,
            general_seen,
            general_headers,
            general_pauses,
            refresh,
            preload_pauses,
            environment,
            restored: std::cell::Cell::new(false),
            map,
            fetch,
            fetch_pending,
            typed,
            typed_pending,
            policies,
            referrers,
            cache,
            modules,
            statuses,
            typed_keys,
            typed_bytes,
            errors,
            receipts,
            reports,
            preloads,
            preload_seen,
            preload_headers,
            _bytes: bytes,
        })
    }
    fn restore_parser_module_snapshot(
        &mut self,
        snapshot: &ParserModuleSnapshot,
    ) -> Result<(), HostError> {
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("parser module context missing".into()))?;
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        if current_environment(scope).map_err(HostError::Execution)? != snapshot.environment {
            return Err(HostError::Execution(
                "parser module checkpoint owner changed".into(),
            ));
        }
        if snapshot.restored.get() {
            return Ok(());
        }
        let id = snapshot.environment.id;
        if let Some(refresh) = &snapshot.refresh {
            context
                .get_slot::<V8BindingState>()
                .ok_or_else(|| HostError::Execution("refresh checkpoint binding missing".into()))?
                .restore_refresh_checkpoint(refresh)
                .map_err(|error| HostError::Execution(error.to_string()))?;
        }
        // A provisional link may have changed an existing uninstantiated module.
        // Such a module was not exposed by evaluation; discard its compiled record
        // and recompile from the retained response under the restored import map.
        let invalidated = snapshot
            .statuses
            .iter()
            .filter_map(|(module, status)| {
                let local = v8::Local::new(scope, module);
                (local.get_status() != *status).then(|| module.clone())
            })
            .collect::<std::collections::HashSet<_>>();
        if let Some(queue) = scope.get_slot::<Rc<RefCell<ModulePreloads>>>() {
            let mut queue = queue.borrow_mut();
            queue.jobs.retain(|job| {
                job.environment.id != id
                    || snapshot.preloads.iter().any(|(node, key, deadline)| {
                        *node == job.target.as_ref().map(|target| target.current().1)
                            && *key == job.key
                            && *deadline == job.deadline
                    })
            });
            queue.seen.retain(|(owner, node), _| {
                *owner != id || snapshot.preload_seen.contains_key(node)
            });
            for (node, key) in &snapshot.preload_seen {
                queue.seen.insert((id, *node), key.clone());
            }
            queue.headers.retain(|(owner, header)| {
                *owner != id || snapshot.preload_headers.contains(header)
            });
        }
        snapshot.local_resource_preloads.restore();
        #[cfg(unix)]
        {
            // Revoke tentative network ownership before the parser drops old consumers.
            // Baseline jobs are paused throughout the checkpoint and keep their exact
            // response/publication state, even when their network response completed.
            if let Some(preloads) = &snapshot.resource_preloads {
                preloads.restore();
            }
        }
        if let Some(queue) = scope.get_slot::<Rc<RefCell<GeneralPreloads>>>() {
            let mut queue = queue.borrow_mut();
            queue.jobs.retain(|job| {
                job.environment.id != id
                    || snapshot
                        .general_preloads
                        .iter()
                        .any(|(node, key, deadline)| {
                            *node == job.target.as_ref().map(|target| target.current().1)
                                && *key == job.key
                                && *deadline == job.deadline
                        })
            });
            queue.seen.retain(|(owner, node), _| {
                *owner != id || snapshot.general_seen.contains_key(node)
            });
            for (node, key) in &snapshot.general_seen {
                queue.seen.insert((id, *node), key.clone());
            }
            queue.headers.retain(|(owner, _)| *owner != id);
            queue.headers.extend(
                snapshot
                    .general_headers
                    .iter()
                    .cloned()
                    .map(|key| (id, key)),
            );
        }
        if let Some(queue) = scope.get_slot_mut::<ModuleReportQueue>() {
            queue
                .jobs
                .retain(|job| job.environment.id != id || snapshot.reports.contains(&job.resolver));
        }
        if let Some(cache) = context.get_slot::<RefCell<ModuleFetchResults>>() {
            let mut cache = cache.borrow_mut();
            cache.0.retain(|key, _| {
                snapshot.fetch.contains(key) || snapshot.fetch_pending.contains(key)
            });
            cache.2.retain(|key| snapshot.fetch_pending.contains(key));
            cache.1.set(cache.1.get().wrapping_add(1));
        }
        if let Some(cache) = context.get_slot::<RefCell<TypedModuleFetchResults>>() {
            let mut cache = cache.borrow_mut();
            cache.0.retain(|key, _| {
                snapshot.typed.contains(key) || snapshot.typed_pending.contains(key)
            });
            cache.2.retain(|key| snapshot.typed_pending.contains(key));
            cache.1.set(cache.1.get().wrapping_add(1));
        }
        if let Some(values) = context.get_slot::<RefCell<ModuleFetchPolicies>>() {
            values.borrow_mut().0 = snapshot.policies.clone();
        }
        if let Some(values) = context.get_slot::<RefCell<ModuleReferrerPolicies>>() {
            values.borrow_mut().0 = snapshot.referrers.clone();
        }
        if let Some(cache) = scope.get_slot_mut::<ModuleCache>() {
            cache.0.retain(|(owner, key), module| {
                *owner != id
                    || ((snapshot.cache.contains(key) || snapshot.fetch_pending.contains(key))
                        && !invalidated.contains(module))
            });
        }
        let mut retained_modules = snapshot
            .modules
            .iter()
            .filter(|module| !invalidated.contains(*module))
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let retained_typed_bytes = context
            .get_slot::<RefCell<TypedModuleFetchResults>>()
            .map(|cache| {
                cache
                    .borrow()
                    .0
                    .values()
                    .filter_map(Option::as_ref)
                    .map(|(_, source)| source.len())
                    .sum::<usize>()
            })
            .unwrap_or(0)
            .max(snapshot.typed_bytes);
        if let Some(cache) = scope.get_slot_mut::<TypedModules>() {
            cache.entries.retain(|(owner, key, kind), value| {
                *owner != id
                    || ((snapshot.typed_keys.contains(&(key.clone(), *kind))
                        || snapshot.typed_pending.contains(&(key.clone(), *kind)))
                        && !value.as_ref().is_some_and(|result| {
                            result
                                .as_ref()
                                .is_ok_and(|module| invalidated.contains(module))
                        }))
            });
            for ((owner, _, _), value) in &cache.entries {
                if *owner == id
                    && let Some(Ok(module)) = value
                {
                    retained_modules.insert(module.clone());
                }
            }
            cache
                .values
                .retain(|(owner, module, _)| *owner != id || retained_modules.contains(module));
            cache.source_bytes.insert(id, retained_typed_bytes);
        }
        if let Some(cache) = scope.get_slot::<ModuleCache>() {
            for ((owner, _), module) in &cache.0 {
                if *owner == id {
                    retained_modules.insert(module.clone());
                }
            }
        }
        let removed = scope
            .get_slot::<ModuleOwners>()
            .map(|owners| {
                owners
                    .0
                    .iter()
                    .filter(|(module, owner)| **owner == id && !retained_modules.contains(*module))
                    .map(|(module, _)| module.clone())
                    .collect::<std::collections::HashSet<_>>()
            })
            .unwrap_or_default();
        if let Some(names) = scope.get_slot_mut::<ModuleNames>() {
            names.0.retain(|module, _| !removed.contains(module));
        }
        if let Some(credentials) = scope.get_slot_mut::<ModuleCredentialRecords>() {
            credentials.0.retain(|module, _| !removed.contains(module));
        }
        if let Some(owners) = scope.get_slot_mut::<ModuleOwners>() {
            owners.0.retain(|module, _| !removed.contains(module));
        }
        if let Some(errors) = context.get_slot::<RefCell<ModuleGraphErrors>>() {
            let mut errors = errors.borrow_mut();
            errors.parsed.retain(|key, _| snapshot.errors.contains(key));
            errors
                .receipts
                .retain(|key, _| snapshot.receipts.contains(key));
        }
        *module_import_map(scope).borrow_mut() = snapshot.map.clone();
        snapshot.restored.set(true);
        snapshot.preload_pauses.borrow_mut().clear();
        snapshot.general_pauses.borrow_mut().clear();
        Ok(())
    }
}

#[cfg(test)]
mod parser_module_snapshot_tests {
    use super::*;
    fn host() -> RawV8Host {
        let mut host = RawV8Host::new_with_base_url_and_heap_limit(
            Url::parse("https://example.test/").unwrap(),
            256 * 1024 * 1024,
        );
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<head></head>")
                .into_runtime()
                .unwrap(),
        ));
        host.install_dom(dom, 1).unwrap();
        host.commit_location("https://example.test/").unwrap();
        host
    }
    fn resolve(host: &mut RawV8Host, specifier: &str) -> Option<String> {
        v8::scope!(let handles,&mut host.isolate);
        let context = v8::Local::new(handles, host.contexts.get(&host.active_realm).unwrap());
        let scope = &mut v8::ContextScope::new(handles, context);
        module_import_map(scope)
            .borrow()
            .resolve(&Url::parse("https://example.test/").unwrap(), specifier)
            .unwrap()
    }
    #[test]
    fn replay_restores_import_map_baseline_and_ack_retry_preserves_reparsed_map() {
        let mut host = host();
        host.register_import_map(r#"{"imports":{"baseline":"./baseline.js"}}"#)
            .unwrap();
        let snapshot = host.capture_parser_module_snapshot().unwrap();
        host.register_import_map(r#"{"imports":{"provisional":"./wrong.js"}}"#)
            .unwrap();
        assert!(resolve(&mut host, "provisional").is_some());
        host.restore_parser_module_snapshot(&snapshot).unwrap();
        assert_eq!(
            resolve(&mut host, "baseline").as_deref(),
            Some("https://example.test/baseline.js")
        );
        assert_eq!(resolve(&mut host, "provisional"), None);
        host.register_import_map(r#"{"imports":{"reparsed":"./correct.js"}}"#)
            .unwrap();
        host.restore_parser_module_snapshot(&snapshot).unwrap();
        assert_eq!(
            resolve(&mut host, "reparsed").as_deref(),
            Some("https://example.test/correct.js")
        );
    }
    #[test]
    fn checkpoint_accounts_for_complete_preload_key_including_media_and_nonce() {
        let mut host = host();
        let baseline = host.capture_parser_module_snapshot().unwrap();
        let key = ModulePreloadKey {
            url: "https://example.test/module.js".into(),
            kind: Some(crate::ModuleType::JavaScript),
            include: false,
            policy: "strict-origin-when-cross-origin".into(),
            integrity: Some("sha256-digest".into()),
            base: "https://example.test/".into(),
            media: "screen and (min-width: 1px)".repeat(128),
            nonce: Some("nonce".repeat(1024)),
            parser_inserted: true,
        };
        let expected = key.retained_bytes() + 256;
        {
            v8::scope!(let handles,&mut host.isolate);
            let context = v8::Local::new(handles, host.contexts.get(&host.active_realm).unwrap());
            let scope = &mut v8::ContextScope::new(handles, context);
            let owner = current_environment(scope).unwrap().id;
            let node = host
                .binding_states
                .get(&host.active_realm)
                .unwrap()
                .document_root();
            module_preloads(scope)
                .borrow_mut()
                .seen
                .insert((owner, node), key);
        }
        let snapshot = host.capture_parser_module_snapshot().unwrap();
        assert_eq!(snapshot._bytes.0 - baseline._bytes.0, expected);
    }
    #[test]
    fn replay_drops_only_new_sticky_fetch_entries() {
        let mut host = host();
        let cache = {
            v8::scope!(let handles,&mut host.isolate);
            let context = v8::Local::new(handles, host.contexts.get(&host.active_realm).unwrap());
            let cache = Rc::new(RefCell::new(ModuleFetchResults::default()));
            cache
                .borrow_mut()
                .retain("before", Some(("before", "export const value=1")), false)
                .unwrap();
            context.set_slot(cache.clone());
            cache
        };
        let snapshot = host.capture_parser_module_snapshot().unwrap();
        cache.borrow_mut().retain("after", None, false).unwrap();
        host.restore_parser_module_snapshot(&snapshot).unwrap();
        assert!(cache.borrow().0.contains_key("before"));
        assert!(!cache.borrow().0.contains_key("after"));
    }
}
