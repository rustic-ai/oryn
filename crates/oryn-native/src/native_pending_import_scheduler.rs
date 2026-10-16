#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct NativePendingImportKey {
    realm: RealmId,
    document: NodeId,
    document_generation: u64,
    stream_epoch: u64,
    owner: NodeId,
    source_revision: u64,
    publication: usize,
    occurrence: u64,
    job_serial: u64,
}
static NEXT_NATIVE_IMPORT_JOB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[cfg(test)]
mod native_pending_import_key_tests {
    use super::*;

    #[test]
    fn equal_rule_ordinals_in_different_stylesheets_do_not_alias() {
        let first = NativePendingImportKey {
            realm: RealmId(0),
            document: NodeId {
                slot: 0,
                generation: 0,
            },
            document_generation: 1,
            stream_epoch: 1,
            owner: NodeId {
                slot: 2,
                generation: 0,
            },
            source_revision: 1,
            publication: 10,
            occurrence: 1,
            job_serial: 1,
        };
        let second = NativePendingImportKey {
            owner: NodeId {
                slot: 3,
                generation: 0,
            },
            publication: 11,
            ..first
        };
        let mut replies = BTreeMap::new();
        replies.insert(first, "first");
        replies.insert(second, "second");
        assert_eq!(replies.len(), 2);
        assert_eq!(replies.remove(&first), Some("first"));
        assert_eq!(replies.remove(&second), Some("second"));
        assert_ne!(
            first,
            NativePendingImportKey {
                job_serial: 2,
                ..first
            }
        );
        assert_ne!(
            first,
            NativePendingImportKey {
                publication: 12,
                ..first
            }
        );
        assert_ne!(
            first,
            NativePendingImportKey {
                document_generation: 2,
                ..first
            }
        );
        assert_ne!(
            first,
            NativePendingImportKey {
                stream_epoch: 2,
                ..first
            }
        );
        assert_ne!(
            first,
            NativePendingImportKey {
                owner: NodeId {
                    slot: 2,
                    generation: 1
                },
                ..first
            }
        );
    }
}
impl NativePendingImportKey {
    fn for_request(request: &NativePendingImportRequest, job_serial: u64) -> Self {
        let identity = request.publication.owner.document;
        Self {
            realm: identity.realm,
            document: identity.document,
            document_generation: identity.document_generation,
            stream_epoch: identity.stream_epoch,
            owner: request.publication.owner.node,
            source_revision: request.publication.owner.revision,
            publication: Rc::as_ptr(&request.publication) as usize,
            occurrence: request.occurrence.0,
            job_serial,
        }
    }
}
impl ParserResourceScheduler {
    #[cfg(unix)]
    fn start_native_pending_import(
        &mut self,
        request_slot: &mut Option<NativePendingImportRequest>,
        host: &mut RawV8Host,
    ) -> Result<(), HostError> {
        let request = request_slot.as_ref().ok_or_else(|| HostError::Execution("pending import request slot empty".into()))?;
        // Return the opaque native request on all pre-dispatch failures.
        let prepared = (|| {
            self.ensure_capacity()?;
            if !request.current()? {
                return Ok(None);
            }
            if self.native_pending_imports.values().any(|job| {
                Rc::ptr_eq(&job.request.publication, &request.publication)
                    && job.request.occurrence == request.occurrence
            }) {
                return Err(HostError::Execution(
                    "duplicate native import job identity".into(),
                ));
            }
            let serial = NEXT_NATIVE_IMPORT_JOB
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |value| value.checked_add(1),
                )
                .map_err(|_| HostError::Execution("native import job identity exhausted".into()))?;
            let key = NativePendingImportKey::for_request(request, serial);
            let identity = request.publication.owner.document;
            let arena = request
                .publication
                .owner
                .arena
                .upgrade()
                .ok_or(HostError::CachedAuthorityExpired)?;
            let previous = host.active_realm();
            host.activate_realm(identity.realm)?;
            let captured =
                ParserFetchContext::capture(host, &arena, request.publication.owner.node);
            let restored = host.activate_realm(previous);
            let fetch = captured?;
            restored?;
            if fetch.identity() != identity {
                return Err(HostError::CachedAuthorityExpired);
            }
            let admission = host
                .native_retained_admission_for_realm(identity.realm)?
                .ok_or_else(|| {
                    HostError::Execution("pending import allocation authority absent".into())
                })?;
            let _receipt_scope = host.retained_allocation_scope(identity.realm)?;
            let endpoint = host.optional_retained_stylesheet_resource_endpoint(identity.realm);
            let standalone = if endpoint.is_none() {
                let previous = host.active_realm();
                host.activate_realm(identity.realm)?;
                let captured = standalone_stylesheet::Loader::capture(
                    host, &BTreeMap::new(), &fetch, self.source_bytes.clone());
                let restored = host.activate_realm(previous);
                let (loader, cancel) = captured?;
                restored?;
                cancel.complete();
                Some(loader)
            } else { None };
            let bytes = [
                request.url.len(),
                request.parent_url.len(),
                request.referrer_policy.len(),
                request.encoding.len(),
            ]
            .into_iter()
            .try_fold(0usize, usize::checked_add)
            .ok_or_else(|| HostError::Execution("pending request source overflow".into()))?;
            let queued_source = ScriptSourceReservation::reserve(&self.source_bytes, bytes)
                .ok_or_else(|| {
                    HostError::Execution("configured queued resource source limit".into())
                })?;
            let retained_bytes = bytes
                .checked_add(std::mem::size_of::<NativePendingImportFetch>())
                .and_then(|value| {
                    value.checked_add(
                        std::mem::size_of::<(NativePendingImportKey, NativePendingImportJob)>()
                            + 6 * std::mem::size_of::<usize>(),
                    )
                })
                .ok_or_else(|| {
                    HostError::Execution("pending request allocation overflow".into())
                })?;
            let request_allocation = admission
                .reserve(retained_bytes)
                .map_err(HostError::Execution)?;
            // Exact copies remain governed by the request's retained descriptor
            // admission; transient source bytes additionally obey queue bounds.
            let work = NativePendingImportFetch {
                endpoint,
                standalone,
                initiator: fetch.stylesheet_initiator()?,
                key,
                document: identity,
                owner_node: request.publication.owner.node,
                publication_revision: request.publication.owner.revision,
                occurrence: request.occurrence,
                url: request.url.clone(),
                parent_url: request.parent_url.clone(),
                referrer_policy: request.referrer_policy.clone(),
                encoding: request.encoding.clone(),
                depth: request.depth,
                cancellation: Arc::new(crate::context_service::ResourceCancellation::default()),
                source_budget: self.source_bytes.clone(),
                queued_source,
                _request_allocation: request_allocation,
            };
            let resource_job = work.endpoint.as_ref()
                .map(|(client, _)| client.track_resource_job())
                .transpose()
                .map_err(|error| HostError::Execution(error.to_string()))?;
            Ok(Some((key, work, resource_job)))
        })();
        let (key, work, resource_job) = match prepared {
            Ok(Some(value)) => value,
            Ok(None) => { request_slot.take(); return Ok(()); },
            Err(error) => return Err(error),
        };
        let request = request_slot.take().expect("validated pending request slot");
        let request_allocation = work._request_allocation.clone();
        let cancellation = work.cancellation.clone();
        self.native_pending_imports.insert(
            key,
            NativePendingImportJob {
                request,
                reply: None,
                failure: None,
                failed: false,
                prepared: None,
                _request_allocation: request_allocation,
                cancellation,
                dependencies: None,
                committed: false,
                _dependency_allocation: None,
                settlement_failures: (None,None),
            },
        );
        let sender = self.sender.clone();
        let lifetime = Arc::downgrade(&self.lifetime);
        spawn_parser_resource(
            sender,
            lifetime,
            resource_job,
            move || Ok(work.execute()),
            move |result| ParserResourceCompletion::NativePendingImport { key, result },
        );
        Ok(())
    }
}
