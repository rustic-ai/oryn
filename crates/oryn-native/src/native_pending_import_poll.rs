impl ParserResourceScheduler {
    fn receive_native_pending_import(
        &mut self,
        key: NativePendingImportKey,
        result: Result<NativePendingImportReply, String>,
    ) -> Result<(), HostError> {
        let Some(job) = self.native_pending_imports.get_mut(&key) else {
            return Ok(());
        };
        match result {
            Ok(reply)
                if reply.key == key
                    && key == NativePendingImportKey::for_request(&job.request, key.job_serial)
                    && reply.document == job.request.publication.owner.document
                    && reply.publication_revision == job.request.publication.owner.revision
                    && reply.occurrence == job.request.occurrence =>
            {
                match reply.result {
                    Ok(source) => job.reply = Some(source),
                    Err(error) => {
                        job.failure = Some(HostError::Execution(error));
                        job.failed = true;
                    }
                }
            }
            Ok(_) => {
                job.failed = true;
                return Err(HostError::Execution(
                    "pending import response owner mismatch".into(),
                ));
            }
            Err(error) => {
                job.failure = Some(HostError::Execution(error));
                job.failed = true;
            }
        }
        Ok(())
    }

    fn poll_native_pending_imports(&mut self, host: &mut RawV8Host) -> Result<(), HostError> {
        let turns = self
            .native_pending_imports
            .len()
            .min(MAX_SCRIPT_COMPLETIONS_PER_SLICE);
        let mut previous_key = None;
        for _ in 0..turns {
            let next = match previous_key {
                None => self.native_pending_imports.keys().next().copied(),
                Some(key) => self
                    .native_pending_imports
                    .range((std::ops::Bound::Excluded(key), std::ops::Bound::Unbounded))
                    .next()
                    .map(|(key, _)| *key),
            };
            let Some(key) = next else {
                break;
            };
            previous_key = Some(key);
            let Some(mut job) = self.native_pending_imports.remove(&key) else {
                continue;
            };
            let Some(arena) = job.request.publication.owner.arena.upgrade() else {
                continue;
            };
            // Retirement is ordinary cancellation, not a failed attempt to
            // refresh a now-unavailable realm's allocation receipt.
            if job.request.publication.owner_publication_retired()? {
                continue;
            }
            let identity = job.request.publication.owner.document;
            let refreshed = host.native_retained_admission_for_realm(identity.realm);
            if let Err(error) = refreshed {
                self.native_pending_imports.insert(key, job);
                return Err(error);
            }
            let _receipt_scope = match host.retained_allocation_scope(identity.realm) {
                Ok(scope) => scope,
                Err(error) => {
                    self.native_pending_imports.insert(key, job);
                    return Err(error);
                }
            };
            let previous = host.active_realm();
            if let Err(error) = host.activate_realm(identity.realm) {
                self.native_pending_imports.insert(key, job);
                return Err(error);
            }
            let result = job.publish(host, &arena);
            let invalidated = host.invalidate_native_retained_allocation(identity.realm);
            let restored = host.activate_realm(previous);
            match result {
                Ok(true) => {
                    invalidated?;
                    restored?;
                }
                Ok(false) => {
                    self.native_pending_imports.insert(key, job);
                    invalidated?;
                    restored?;
                }
                Err(error) => {
                    self.native_pending_imports.insert(key, job);
                    return Err(error);
                }
            }
        }
        Ok(())
    }
}
