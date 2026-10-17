// Included in the scheduler compilation unit so its private job state remains
// private; no external API exposes mutable publication flags or result slots.
pub(super) fn current_implicit_font_job(job:&StylesheetPreparation,host:&RawV8Host,realm:RealmId)->bool{
    job.identity.realm==realm&&job.font_family.is_some()&&job.live(host)
}
impl RustExecutableDocument {
    pub(super) fn synchronize_initial_implicit_fonts(&mut self,realm:RealmId)->Result<(),HostError>{
        self.agent.host.synchronize_public_font_set(realm)?;
        for timeline in self.css_font_timelines.iter_mut().filter(|timeline|timeline.identity.realm==realm){
            if !timeline.valid(&self.agent.host){continue;}
            if !timeline.implicit_started && timeline.implicit_terminal.is_none() && !timeline.loaded {
                if let (Some(rule),Some(epoch))=(&timeline.font.native_rule,&timeline.font.retained){
                    let identity=epoch.publication.graph.borrow().connected_font_context(rule).map_err(HostError::Execution)?.identity();
                    let tokens=self.agent.host.publish_implicit_font_transition(realm,identity)?;
                    if !tokens.tokens.is_empty(){timeline.implicit_tokens=Some(tokens);timeline.implicit_started=true;}
                }
            }
            if let (Some(success),Some(tokens))=(timeline.implicit_terminal.or(timeline.loaded.then_some(true)),&timeline.implicit_tokens){
                self.agent.host.complete_implicit_font_tokens(realm,&tokens.tokens,success)?;
                timeline.implicit_tokens=None;
            }
        }
        Ok(())
    }
    fn finish_implicit_font_backing(&mut self,job:&mut StylesheetPreparation)->Result<(),HostError>{
        let Some(backing)=job.prepared_implicit_backing.as_ref()else{return Ok(())};
        if !job.live(&self.agent.host)||!backing.epoch.current(){return Err(HostError::CachedAuthorityExpired);}
        let provider=self.agent.host.native_layout_provider_in_realm(job.identity.realm).ok_or(HostError::CachedAuthorityExpired)?;
        let store=provider.native_backend().and_then(|backend|backend.downcast_ref::<LiveLayoutProvider>())
            .ok_or(HostError::CachedAuthorityExpired)?.implicit_font_backing.try_borrow().map_err(|_|HostError::Execution("implicit backing commit slot borrow".into()))?
            .as_ref().cloned().ok_or(HostError::CachedAuthorityExpired)?;
        commit_implicit_font_backing(&store,backing.clone())?;
        drop(job.prepared_implicit_backing.take());
        Ok(())
    }
}
