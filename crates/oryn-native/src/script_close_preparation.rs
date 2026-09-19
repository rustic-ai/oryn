// Target-owned, bounded one-admission script-close continuation. Parent polls
// outside its caller IPC so shared worker actors never query one another inline.
struct ScriptCloseJob{token:u64,expected:oryn_js::worker_wire::Creator,departure:crate::host::NativeScriptClose,result:Option<crate::context_service::ScriptClosePreparation>}
impl PageState{
    fn script_close_owns_departure(&self)->bool{self.script_close_preparation.as_ref().is_some_and(|job|!matches!(job.result,Some(crate::context_service::ScriptClosePreparation::Canceled)))}
    fn script_close_departure_completed(&self)->bool{self.script_close_preparation.as_ref().is_some_and(|job|matches!(job.result,Some(crate::context_service::ScriptClosePreparation::Allowed(_))))}

    fn poll_script_close_preparation(&mut self,token:u64,expected:oryn_js::worker_wire::Creator)->Result<crate::context_service::ScriptClosePreparation,RuntimeError>{
        use crate::context_service::ScriptClosePreparation;
        if token==0{return Err(RuntimeError::ProtocolViolation("zero script-close admission token".into()))}
        // The capability belongs to the traversable. Its admitted Creator is
        // authentication/replay metadata, not the Document selected by the task.
        if let Some(job)=&self.script_close_preparation{
            if job.token==token&&job.expected==expected{if let Some(result)=&job.result{return Ok(result.clone())}}
            else if matches!(&job.result,Some(ScriptClosePreparation::Canceled)){self.script_close_preparation=None}
            else{return Ok(ScriptClosePreparation::Pending)}
        }
        // Earlier traversal departure/publication owns the root until its
        // retained commit finishes. Network preparation itself does not delay close.
        if self.script_close_preparation.is_none()&&self.pending_navigation_transaction.as_ref().is_some_and(|transaction|matches!(transaction.phase,NavigationTransactionPhase::Departure{..}|NavigationTransactionPhase::Publication{..})){return Ok(ScriptClosePreparation::Pending)}
        let executable=self.executable.as_mut().ok_or(RuntimeError::EvaluationUnavailable)?;
        if self.script_close_preparation.is_none(){
            let current=executable.script_close_history().map_err(map_host_error)?.creator;
            let departure=executable.prepare_script_close(current).map_err(map_host_error)?;
            self.script_close_preparation=Some(ScriptCloseJob{token,expected,departure,result:None});
        }
        let job=self.script_close_preparation.as_mut().unwrap();if let Some(result)=&job.result{return Ok(result.clone())}
        let outcome=match executable.poll_script_close(&mut job.departure).map_err(map_host_error)?{None=>ScriptClosePreparation::Pending,Some(false)=>ScriptClosePreparation::Canceled,Some(true)=>{
            let current=executable.script_close_history().map_err(map_host_error)?;
            ScriptClosePreparation::Allowed(current)
        }};
        if !matches!(outcome,ScriptClosePreparation::Pending){job.result=Some(outcome.clone());}Ok(outcome)
    }
}

#[cfg(test)]
mod retained_script_close_tests{
 use super::*;
 #[test]
 fn queued_close_selects_current_document_and_beforeunload_cannot_replace_it(){
  let context=Browser::in_process_with_config(Default::default()).new_context();
  let handle=context.new_page();let id=handle.page_id();
  let client=crate::context_service::SharedContextClient::direct(context.clone(),id);
  let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),None,Some(client));
  page.load_document("https://close.test/".into(),"<body>original</body>",None,None,HistoryUpdate::Push,&BTreeMap::new(),Vec::new()).unwrap();
  let admitted=page.executable.as_ref().unwrap().script_close_history().unwrap().creator;
  page.load_document("https://close.test/new".into(),"<body>current</body>",None,None,HistoryUpdate::Push,&BTreeMap::new(),Vec::new()).unwrap();
  page.executable.as_mut().unwrap().evaluate("globalThis.closedEvents=[];onbeforeunload=()=>{closedEvents.push('before');document.open();document.write('<body>replacement</body>');document.close()};onpagehide=()=>closedEvents.push('hide');onunload=()=>closedEvents.push('unload');true").unwrap();
  assert!(matches!(page.poll_script_close_preparation(1,admitted.clone()).unwrap(),crate::context_service::ScriptClosePreparation::Allowed(_)));
  assert!(matches!(page.poll_script_close_preparation(1,admitted).unwrap(),crate::context_service::ScriptClosePreparation::Allowed(_)));
  assert_eq!(page.executable.as_mut().unwrap().evaluate("closedEvents.join(',')+':'+document.body.textContent").unwrap(),"before,hide,unload:current");
 }
 #[test]
 fn close_waits_for_earlier_departure_but_freezes_uncommitted_preparation_after_ownership(){
  struct NoFetch;impl NetworkBroker for NoFetch{fn send(&self,_request:NetworkRequest)->Result<NetworkResponse,crate::network::NetworkError>{panic!("close must not wait for or dispatch prepared network work")}}
  let broker=SharedNetworkBroker::new(NoFetch);
  let context=Browser::in_process_with_config(Default::default()).new_context();
  let handle=context.new_page();let id=handle.page_id();
  let client=crate::context_service::SharedContextClient::direct(context.clone(),id);
  let mut page=PageState::new(id,None,BrowserConfig::default(),ContextOptions::default(),Some(broker),Some(client));
  page.load_document("https://close.test/source".into(),"<body>source</body>",None,None,HistoryUpdate::Push,&BTreeMap::new(),Vec::new()).unwrap();
  let creator=page.executable.as_ref().unwrap().script_close_history().unwrap().creator;
  page.begin_navigation_resource("https://close.test/destination".into(),HistoryUpdate::Push,None,None,None,None).unwrap();
  page.pending_navigation_transaction.as_mut().unwrap().phase=NavigationTransactionPhase::Departure{input:RootNavigationInput::Buffered{media_type:None,bytes:Vec::new()},job:None,retry:None};
  assert!(matches!(page.poll_script_close_preparation(1,creator.clone()).unwrap(),crate::context_service::ScriptClosePreparation::Pending));assert!(page.script_close_preparation.is_none());
  // The same still-uncommitted request has no traversal ownership in Prelude.
  page.pending_navigation_transaction.as_mut().unwrap().phase=NavigationTransactionPhase::Prelude;
  assert!(matches!(page.poll_script_close_preparation(1,creator).unwrap(),crate::context_service::ScriptClosePreparation::Allowed(_)));
  assert!(page.script_close_owns_departure());assert!(page.poll_navigation_transaction().is_none());assert!(matches!(page.pending_navigation_transaction.as_ref().unwrap().phase,NavigationTransactionPhase::Prelude));
  // Cancellation is the only live outcome that releases this traversal fence.
  page.script_close_preparation.as_mut().unwrap().result=Some(crate::context_service::ScriptClosePreparation::Canceled);assert!(!page.script_close_owns_departure());assert!(!page.script_close_departure_completed());
 }

}
