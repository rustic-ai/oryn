// Session-owned native evidence lookup; callers cannot choose child identities.
impl BrowserContext {
    #[cfg(unix)]
    pub fn parent_support_snapshot(&self,page:PageId)->Result<Option<serde_json::Value>,RuntimeError>{
        // Same fixed bound as canonical registration, no GET heap snapshot.
        let mut scopes:[Option<Arc<crate::context_service::SocketLifecycle>>;1024]
            =std::array::from_fn(|_|None);
        {
            let registered=self.inner.socket_lifecycles.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
            if registered.len()>scopes.len(){return Err(RuntimeError::NavigationCancelled);}
            for (slot,scope) in scopes.iter_mut().zip(registered.iter()){
                *slot=scope.upgrade().filter(|scope|scope.page()==page);
            }
        }
        let mut receipt=None;
        for scope in scopes.into_iter().flatten(){
            if let Some(current)=scope.support_current(self,page)?{
                if receipt.replace(current).is_some(){return Err(RuntimeError::ProtocolViolation(
                    "duplicate current support scope".into()));}
            }
        }
        let Some(receipt)=receipt else{return Ok(None);};
        let hex=|bytes:&[u8;32]|->Result<String,RuntimeError>{
            let mut value=String::new();
            value.try_reserve_exact(64).map_err(|_|RuntimeError::WorkerLimitExceeded{
                resource:"support export serialization".into()})?;
            const DIGITS:&[u8;16]=b"0123456789abcdef";
            for byte in bytes{value.push(DIGITS[(byte>>4)as usize]as char);value.push(DIGITS[(byte&15)as usize]as char);}
            Ok(value)
        };
        let mut imports=Vec::new();
        imports.try_reserve_exact(6).map_err(|_|RuntimeError::WorkerLimitExceeded{
            resource:"support import serialization".into()})?;
        for(path,sha256)in crate::support_descriptor::IMPORTS{
            imports.push(serde_json::json!({"path":path,"sha256":sha256}));
        }
        Ok(Some(serde_json::json!({"schema":1,"current":true,"eof":true,
            "candidate":hex(&receipt.candidate)?,"descriptor":hex(&receipt.descriptor)?,
            "parent":receipt.parent,"child":receipt.child,
            "body_sha256":hex(&receipt.body_sha256)?,"body_bytes":receipt.body_bytes,
            "url_sha256":hex(&receipt.url_sha256)?,"imports":imports })))
    }
}
