// Exact completed source epochs, not a wrapping sum or DOM mutation proxy.
struct FontQuiescenceStamp {
    identity:ParserDocumentIdentity,
    public_cycle:u64,
    layout_revision:Option<u64>,
    binding_inputs:oryn_js::v8_bindings::font_loading_transport::FontLayoutInputs,
    sources:Vec<(NodeId,u64,Option<dynamic_stylesheet_preparation::RetainedFontDemandEpoch>)>,
    serial:u64,
    _allocation:Arc<dyn Send+Sync>,
}
impl FontQuiescenceStamp {
    fn same(&self,other:&Self)->bool {
        self.identity==other.identity && self.public_cycle==other.public_cycle
            && self.layout_revision==other.layout_revision && self.binding_inputs.states==other.binding_inputs.states
            && self.sources.len()==other.sources.len()
            && self.sources.iter().zip(&other.sources).all(|((node,revision,epoch),(other_node,other_revision,other_epoch))|
                node==other_node && revision==other_revision && match(epoch,other_epoch){
                    (None,None)=>true,(Some(epoch),Some(other))=>epoch.same(other),_=>false,
                })
    }
}
impl RustExecutableDocument {
    fn capture_font_quiescence(&mut self,realm:RealmId)->Result<FontQuiescenceStamp,HostError>{
        let dom=self.agent.host.automation_dom(realm).ok_or(HostError::CachedAuthorityExpired)?;
        let document=self.agent.host.retained_binding_state(realm)?.document_root();
        let identity=ParserFetchContext::capture(&self.agent.host,&dom,document)?.identity();
        let count=self.css_dependency_sources.iter().filter(|source|source.realm==realm).count();
        let shared=self.agent.host.native_retained_admission_for_realm(realm)?.ok_or(HostError::CachedAuthorityExpired)?;
        let _scope=self.agent.host.retained_allocation_scope(realm)?;
        let allocation=shared.reserve(count.checked_mul(std::mem::size_of::<(NodeId,u64,Option<dynamic_stylesheet_preparation::RetainedFontDemandEpoch>)>())
            .ok_or_else(||HostError::Execution("font quiescence snapshot overflow".into()))?).map_err(HostError::Execution)?;
        if ParserFetchContext::capture(&self.agent.host,&dom,document)?.identity()!=identity
            || self.css_dependency_sources.iter().filter(|source|source.realm==realm).count()!=count {
            return Err(HostError::CachedAuthorityExpired);
        }
        let mut sources=Vec::new();sources.try_reserve_exact(count).map_err(|_|HostError::Execution("font quiescence snapshot allocation".into()))?;
        if sources.capacity()>count{return Err(HostError::Execution("font quiescence source capacity exceeded admission".into()));}
        for source in self.css_dependency_sources.iter().filter(|source|source.realm==realm){
            sources.push((source.batch.node,source.batch.revision,source.evaluated_font.clone()));
        }
        let state=self.agent.host.retained_binding_state(realm)?;
        let public_cycle=state.native_font_cycle();
        let binding_inputs=state.native_font_layout_inputs().map_err(|error|HostError::Execution(error.to_string()))?;
        let layout_revision=self.agent.host.native_layout_provider_in_realm(realm).and_then(|provider|provider.layout_revision());
        if ParserFetchContext::capture(&self.agent.host,&dom,document)?.identity()!=identity{return Err(HostError::CachedAuthorityExpired);}
        Ok(FontQuiescenceStamp{identity,public_cycle,layout_revision,binding_inputs,sources,serial:0,_allocation:allocation})
    }
    fn refresh_font_ready_layout(&mut self,realm:RealmId)->Result<(),HostError>{
        if !self.fonts_quiescent(realm){return Ok(());}
        let mut before=self.capture_font_quiescence(realm)?;
        if before.layout_revision.is_some() && self.font_ready_stamps.iter().find(|(id,_)|*id==realm).is_some_and(|(_,old)|old.same(&before)){return Ok(());}
        let serial=self.next_font_quiescence.checked_add(1).ok_or_else(||HostError::Execution("font quiescence identity exhausted".into()))?;
        self.next_font_quiescence=serial;
        before.serial=serial;
        self.agent.host.demand_public_font_layout(realm,before.serial)?;
        self.agent.host.native_css_resource_layout(realm)?;
        let after=self.capture_font_quiescence(realm)?;
        if before.same(&after) && self.fonts_quiescent(realm){
            let position=self.font_ready_stamps.iter().position(|(id,_)|*id==realm);
            let mut prepared=None;
            if position.is_none(){
                let target=self.font_ready_stamps.len().checked_add(1).ok_or_else(||HostError::Execution("font ready container overflow".into()))?;
                let shared=self.agent.host.native_retained_admission_for_realm(realm)?.ok_or(HostError::CachedAuthorityExpired)?;
                let _scope=self.agent.host.retained_allocation_scope(realm)?;
                let charge=shared.reserve(target.checked_mul(std::mem::size_of::<(RealmId,FontQuiescenceStamp)>()).ok_or_else(||HostError::Execution("font ready container bytes overflow".into()))?).map_err(HostError::Execution)?;
                let mut replacement=Vec::new();replacement.try_reserve_exact(target).map_err(|_|HostError::Execution("font ready container allocation".into()))?;
                if replacement.capacity()>target{return Err(HostError::Execution("font ready container exceeded admission".into()));}
                prepared=Some((replacement,charge));
            }
            if !before.same(&self.capture_font_quiescence(realm)?)||!self.fonts_quiescent(realm){return Ok(());}
            if !self.agent.host.commit_public_font_layout(realm,before.serial)?{return Ok(());}
            if let Some(position)=position{let old=std::mem::replace(&mut self.font_ready_stamps[position].1,before);drop(old);}
            else if let Some((mut replacement,charge))=prepared{
                replacement.append(&mut self.font_ready_stamps);replacement.push((realm,before));
                let old=std::mem::replace(&mut self.font_ready_stamps,replacement);drop(old);
                let old_charge=self.font_ready_stamp_capacity.replace(charge);drop(old_charge);
            }
        }
        Ok(())
    }
    fn fonts_quiescent(&self,realm:RealmId)->bool {
        !self.public_font_jobs.iter().any(|job|job.realm==realm && self.agent.host.public_font_request_current(job.realm,job.token))
            && !self.dynamic_stylesheets.iter().any(|job|
                dynamic_stylesheet_preparation::current_implicit_font_job(job,&self.agent.host,realm))
            && self.css_dependency_sources.iter().filter(|source|source.realm==realm)
                .all(|source|source.evaluated.is_some()
                    && dynamic_stylesheet_preparation::retained_font_evaluation_current(&self.agent.host,source))
    }
}
