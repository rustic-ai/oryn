//! Semantic graph synchronization and retained observation snapshots.

use super::*;

impl PageState {
    #[cfg(feature = "v8-host")]
    pub(super) fn sync_executable_semantics(&mut self) -> Result<(), RuntimeError> {
        let _timing = crate::native_timing::stage("page.semantic_sync");
        let _close_phase = crate::ipc_progress::close_admission::phase("semantic_projection");
        let Some(executable) = self.executable.as_ref() else {
            return Ok(());
        };
        #[cfg(test)]
        semantic_sync_faults::check(semantic_sync_faults::Stage::Snapshot)?;
        let document = {
            let _timing = crate::native_timing::stage("page.semantic_snapshot");
            let document = executable
                .semantic_snapshot()
                .map_err(|error| RuntimeError::Semantic(format!("snapshot: {error}")))?;
            crate::native_timing::count("page.semantic_snapshot", 1);
            document
        };
        let batch = executable.semantic_mutations();
        // Stage graph and delta journal together. A native control read may
        // fail after visiting earlier nodes; none of that partial state can be
        // published or consume the DOM journal on failure.
        let mut graph = self.semantic_graph.clone();
        if batch.truncated {
            graph.reset(&document.dom, self.document_generation);
        } else if !batch.mutations.is_empty() {
            graph.apply_mutations(&document.dom, &batch.mutations);
        }
        #[cfg(test)]
        semantic_sync_faults::check(semantic_sync_faults::Stage::Controls)?;
        executable
            .populate_semantic_control_state(&mut graph)
            .map_err(|error| RuntimeError::Semantic(format!("native controls: {error}")))?;
        let interactive = executable.document_interactive();
        let complete = executable.document_load_complete();
        if self.frame_scope.is_none()
            && let Some(update) = executable.take_navigation_history_update()
        {
            self.apply_navigation_history_update(update)?;
        }
        if self.pending_navigation_response.is_none()
            && !self.lifecycle.contains(&LifecycleState::DomContentLoaded)
            && interactive
        {
            if self.navigation_committed
                && let Some(navigation) = &self.active_navigation
            {
                navigation
                    .publish(crate::navigation_control::NavigationEvent::Interactive {
                        document_generation: self.document_generation,
                    })
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
            }
            self.lifecycle.push(LifecycleState::DomContentLoaded);
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::LifecycleChanged {
                    state: LifecycleState::DomContentLoaded,
                },
            );
        }
        if self.pending_navigation_response.is_none()
            && self.lifecycle.contains(&LifecycleState::DomContentLoaded)
            && !self.lifecycle.contains(&LifecycleState::Load)
            && complete
        {
            if self.navigation_committed {
                if let Some(navigation) = &self.active_navigation {
                    navigation
                        .publish(crate::navigation_control::NavigationEvent::Complete {
                            document_generation: self.document_generation,
                        })
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                }
                self.active_navigation.take();
                self.navigation_committed = false;
            }
            self.lifecycle.push(LifecycleState::Load);
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::LifecycleChanged {
                    state: LifecycleState::Load,
                },
            );
            #[cfg(unix)]
            self.record_committed_navigation_progress(NavigationReadinessStage::LoadReady);
        }
        self.executable
            .as_mut()
            .ok_or(RuntimeError::EvaluationUnavailable)?
            .commit_semantic_mutations(batch.next);
        self.semantic_publication_pending |=
            graph.semantic_revision() != self.semantic_graph.semantic_revision();
        self.semantic_graph = graph;
        self.document = Some(document);
        // Publish asynchronous resource/script diagnostics only after the
        // fallible projection work commits. A retry must neither lose nor
        // duplicate these entries; capability inspection retains the full list.
        for diagnostic in self
            .executable
            .as_mut()
            .unwrap()
            .take_unpublished_diagnostics()
        {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::ScriptDiagnostic {
                    capability: diagnostic.capability,
                    detail: diagnostic.detail.unwrap_or_default(),
                },
            );
        }
        crate::native_timing::count("page.semantic_sync", 1);
        Ok(())
    }

    /// A failed background projection cannot turn the next read into a stale
    /// success. Retry retained mutation custody before publishing observations.
    pub(super) fn refresh_semantic_observation(&mut self) -> Result<(), RuntimeError> {
        #[cfg(feature = "v8-host")]
        {
            self.sync_executable_semantics()?;
            if self.semantic_publication_pending {
                self.refresh_layout()?;
                self.revision = self.revision.checked_add(1).ok_or_else(|| {
                    RuntimeError::Semantic("observation revision exhausted".into())
                })?;
                self.record_snapshot();
            }
        }
        Ok(())
    }

    pub(super) fn observation(&self, profile: ProjectionProfile) -> Observation {
        let Some(document) = &self.document else {
            let mut observation = empty_observation(
                self.page_id,
                self.opener_page_id,
                "about:blank".into(),
                String::new(),
                self.document_generation,
            );
            observation.profile = profile;
            return observation;
        };
        Observation {
            contract_version: CONTRACT_VERSION,
            page: PageInfo {
                page_id: self.page_id,
                opener_page_id: self.opener_page_id,
                url: self.url.clone(),
                title: document_title(document),
                document_generation: self.document_generation,
            },
            revision: Revision(self.revision),
            profile,
            source: ObservationSource::NativeDom,
            nodes: self.semantic_graph.nodes(profile),
            capabilities: self.capabilities(),
            diagnostics: document.errors.clone(),
            execution_domain: ExecutionDomain::Native,
        }
    }

    pub(super) fn semantic_observation(
        &self,
        profile: ProjectionProfile,
    ) -> oryn_common::v5::SemanticObservation {
        let observation = self.observation(profile);
        self.semantic_graph.observation_v5(observation)
    }

    pub(super) fn semantic_observation_v6(
        &mut self,
        options: oryn_common::v6::SemanticObservationOptions,
    ) -> Result<oryn_common::v6::SemanticObservation, RuntimeError> {
        if options.geometry != oryn_common::v6::GeometryPolicy::Omit {
            #[cfg(feature = "v8-host")]
            if self.layout.is_none() {
                self.refresh_layout()?;
            }
        }
        let base = self.observation(options.profile);
        #[cfg(feature = "v8-host")]
        let geometry = self.layout.as_ref().map(|layout| &layout.geometry);
        #[cfg(not(feature = "v8-host"))]
        let geometry = None;
        self.semantic_graph
            .observation_v6(base, &options, geometry)
            .map_err(RuntimeError::Semantic)
    }

    pub(super) fn record_snapshot(&mut self) {
        self.semantic_publication_pending = false;
        let v6_changes = self.semantic_graph.take_v6_changes();
        let changes = crate::semantic::SemanticGraphChanges {
            upserted: v6_changes
                .upserted_nodes
                .iter()
                .map(|node| node.semantic.clone())
                .collect(),
            removed: v6_changes.removed_nodes.clone(),
        };
        let bytes = serde_json::to_vec(&v6_changes).map_or(0, |encoded| encoded.len());
        self.semantic_deltas.insert(self.revision, changes);
        if let Some(previous) = self.semantic_v6_deltas.insert(self.revision, v6_changes) {
            self.semantic_delta_bytes = self
                .semantic_delta_bytes
                .saturating_sub(serde_json::to_vec(&previous).map_or(0, |encoded| encoded.len()));
        }
        self.semantic_delta_bytes = self.semantic_delta_bytes.saturating_add(bytes);
        while self.semantic_deltas.len() > self.config.max_revision_snapshots
            || self.semantic_delta_bytes > self.config.max_revision_bytes
        {
            let Some(oldest) = self.semantic_deltas.keys().next().copied() else {
                break;
            };
            // Retain the current revision even when one unusually large
            // observation exceeds the byte ceiling. Older delta bases fail
            // explicitly rather than allowing memory to grow without limit.
            if oldest == self.revision && self.semantic_deltas.len() == 1 {
                break;
            }
            self.semantic_deltas.remove(&oldest);
            if let Some(removed) = self.semantic_v6_deltas.remove(&oldest) {
                self.semantic_delta_bytes = self.semantic_delta_bytes.saturating_sub(
                    serde_json::to_vec(&removed).map_or(0, |encoded| encoded.len()),
                );
            }
        }
    }

    pub(super) fn semantic_delta_from(
        &self,
        from: Revision,
    ) -> Result<oryn_common::v6::SemanticDelta, RuntimeError> {
        if from.0 > self.revision {
            return Err(RuntimeError::UnknownRevision(from));
        }
        if from.0 < self.revision
            && self
                .semantic_v6_deltas
                .keys()
                .next()
                .is_none_or(|oldest| from.0.saturating_add(1) < *oldest)
        {
            return Err(RuntimeError::UnknownRevision(from));
        }
        let mut nodes = BTreeMap::new();
        let mut removed_nodes = BTreeSet::new();
        let mut accessibility = BTreeMap::new();
        let mut removed_accessibility = BTreeSet::new();
        let mut structures = BTreeMap::new();
        let mut removed_structures = BTreeSet::new();
        for changes in self
            .semantic_v6_deltas
            .range((
                std::ops::Bound::Excluded(from.0),
                std::ops::Bound::Included(self.revision),
            ))
            .map(|(_, changes)| changes)
        {
            for reference in &changes.removed_nodes {
                nodes.remove(reference);
                removed_nodes.insert(*reference);
            }
            for node in &changes.upserted_nodes {
                removed_nodes.remove(&node.semantic.semantic_ref);
                nodes.insert(node.semantic.semantic_ref, node.clone());
            }
            for reference in &changes.removed_accessibility {
                accessibility.remove(reference);
                removed_accessibility.insert(*reference);
            }
            for node in &changes.upserted_accessibility {
                removed_accessibility.remove(&node.semantic_ref);
                accessibility.insert(node.semantic_ref, node.clone());
            }
            for id in &changes.removed_structures {
                structures.remove(id);
                removed_structures.insert(id.clone());
            }
            for structure in &changes.upserted_structures {
                removed_structures.remove(&structure.structure_id);
                structures.insert(structure.structure_id.clone(), structure.clone());
            }
        }
        Ok(oryn_common::v6::SemanticDelta {
            contract_version: oryn_common::v6::CONTRACT_VERSION,
            page_id: self.page_id,
            from_revision: from,
            to_revision: Revision(self.revision),
            semantic_revision: self.semantic_graph.semantic_revision(),
            changes: oryn_common::v6::SemanticGraphChanges {
                upserted_nodes: nodes.into_values().collect(),
                removed_nodes: removed_nodes.into_iter().collect(),
                upserted_accessibility: accessibility.into_values().collect(),
                removed_accessibility: removed_accessibility.into_iter().collect(),
                upserted_structures: structures.into_values().collect(),
                removed_structures: removed_structures.into_iter().collect(),
            },
            diagnostics: Vec::new(),
        })
    }

    pub(super) fn delta_from(&self, from: Revision) -> Result<ObservationDelta, RuntimeError> {
        if from.0 > self.revision {
            return Err(RuntimeError::UnknownRevision(from));
        }
        if from.0 == 0 {
            return Ok(ObservationDelta {
                contract_version: CONTRACT_VERSION,
                page_id: self.page_id,
                from_revision: from,
                to_revision: Revision(self.revision),
                upserted: self.semantic_graph.nodes(ProjectionProfile::Compact),
                removed: Vec::new(),
            });
        }
        if from.0 < self.revision
            && self
                .semantic_deltas
                .keys()
                .next()
                .is_none_or(|oldest| from.0.saturating_add(1) < *oldest)
        {
            return Err(RuntimeError::UnknownRevision(from));
        }
        let mut upserted = BTreeMap::new();
        let mut removed = BTreeSet::new();
        for (_, changes) in self.semantic_deltas.range((
            std::ops::Bound::Excluded(from.0),
            std::ops::Bound::Included(self.revision),
        )) {
            for semantic_ref in &changes.removed {
                upserted.remove(semantic_ref);
                removed.insert(*semantic_ref);
            }
            for node in &changes.upserted {
                removed.remove(&node.semantic_ref);
                upserted.insert(node.semantic_ref, node.clone());
            }
        }
        let mut upserted = upserted.into_values().collect::<Vec<_>>();
        upserted.sort_by_key(|node| node.document_order);
        Ok(ObservationDelta {
            contract_version: CONTRACT_VERSION,
            page_id: self.page_id,
            from_revision: from,
            to_revision: Revision(self.revision),
            upserted,
            removed: removed.into_iter().collect(),
        })
    }
}
