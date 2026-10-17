// Included in fixture.rs: exact canonical selected launch, then fixed issuer.
impl Fixture {
    #[cfg(unix)]
    pub fn load_admitted(path:&Path, expected:&str, candidate_path:&Path,
        candidate_raw:&str,candidate:&str,runtime:&str)->Result<Self,CommandError>{
        let admitted=verify_support_candidate_launch(candidate_path,candidate_raw,candidate,runtime)?;
        if !path.is_absolute()||!hash_valid(expected){return Err(invalid("invalid support fixture path"));}
        let root_path=path.parent().ok_or_else(||invalid("missing support fixture root"))?
            .canonicalize().map_err(|_|invalid("invalid support fixture root"))?;
        let root=open_directory(&root_path)?;
        let bytes=read_bounded(open_at(&root,path.file_name().ok_or_else(||invalid("missing fixture name"))?,false)?,MAX_ARTIFACT_BYTES)?;
        if digest(&bytes)!=expected{return Err(invalid("support fixture configuration changed"));}
        let contract:Contract=serde_json::from_slice(&bytes).map_err(|_|invalid("invalid support fixture configuration"))?;
        let support=contract.parent_support.as_ref().map(|record|
            issue_support_fixture(&root,record,&admitted)).transpose()?;
        drop(open_artifact(&root,&contract.tls.certificate)?);
        drop(open_artifact(&root,&contract.tls.key)?);
        let ca=read_bounded(open_artifact(&root,&contract.tls.ca)?,MAX_ARTIFACT_BYTES)?;
        if digest(&ca)!=contract.tls.ca.sha256{return Err(invalid("support fixture CA changed"));}
        let mut fixture=Self::from_contract(contract,ca)?;
        fixture.support=support;
        Ok(fixture)
    }
    #[cfg(not(unix))]
    pub fn load_admitted(_path:&Path,_expected:&str,_candidate_path:&Path,
        _candidate_raw:&str,_candidate:&str,_runtime:&str)->Result<Self,CommandError>{
        Err(invalid("support fixture secure loading unavailable"))
    }
}
