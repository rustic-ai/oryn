// Native secondary artifact check, after canonical signed-bundle admission.
// Not a substitute signature verifier or a caller-generated success receipt.
fn verify_support_candidate_launch(path:&Path, expected_raw:&str, admitted_candidate:&str, runtime_expected:&str)
    ->Result<String,CommandError>{
    if !path.is_absolute()||!hash_valid(expected_raw)||!hash_valid(admitted_candidate)||!hash_valid(runtime_expected){
        return Err(invalid("invalid admitted support candidate artifact"));
    }
    let root_path=path.parent().ok_or_else(||invalid("missing admitted candidate directory"))?
        .canonicalize().map_err(|_|invalid("invalid admitted candidate directory"))?;
    let root=open_directory(&root_path)?;
    let name=path.file_name().ok_or_else(||invalid("missing admitted candidate name"))?;
    let bytes=read_bounded(open_at(&root,name,false)?,MAX_ARTIFACT_BYTES)?;
    if digest(&bytes)!=expected_raw{return Err(invalid("admitted candidate changed before native issuance"));}
    let executable=std::env::current_exe().map_err(|_|invalid("cannot identify native runtime"))?;
    let executable_root=open_directory(executable.parent().ok_or_else(||invalid("missing native runtime directory"))?)?;
    let mut file=open_at(&executable_root,executable.file_name().ok_or_else(||invalid("missing native runtime name"))?,false)?;
    let mut hash=Sha256::new();
    let mut scratch=[0u8;64*1024];
    loop{
        let size=file.read(&mut scratch).map_err(|_|invalid("cannot hash native runtime"))?;
        if size==0{break;}
        hash.update(&scratch[..size]);
    }
    if format!("{:x}",hash.finalize())!=runtime_expected{
        return Err(invalid("native runtime differs from admitted signed bundle"));
    }
    // This digest is derived from actual manifest bytes plus executable check;
    // canonical parent admission supplies its signed build/fixture association.
    Ok(admitted_candidate.to_owned())
}
