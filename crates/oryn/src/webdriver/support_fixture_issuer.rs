// Included in webdriver/fixture.rs. Read only hash-bound pinned source closure.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportFixtureContract {
    schema: u32,
    revision: String,
    candidate: String,
    descriptor: String,
    parent_url: String,
    child_url: String,
    parent: Artifact,
    child: Artifact,
    imports: Vec<Artifact>,
}

#[cfg(unix)]
fn issue_support_fixture(root: &File, record: &Artifact, admitted_candidate:&str)
    -> Result<oryn_native::PinnedSupportDescriptor, CommandError> {
    let bytes = read_bounded(open_artifact(root, record)?, MAX_ARTIFACT_BYTES)?;
    if digest(&bytes) != record.sha256 { return Err(invalid("support contract hash mismatch")); }
    let contract: SupportFixtureContract = serde_json::from_slice(&bytes)
        .map_err(|_| invalid("invalid support fixture contract"))?;
    if contract.schema != 1 || contract.revision != REVISION || contract.imports.len() != 6 {
        return Err(invalid("support fixture closure mismatch"));
    }
    validate_issued_candidate(&contract.candidate,admitted_candidate)?;
    fn fixed_hash(value: &str) -> Result<[u8;32], CommandError> {
        if !hash_valid(value) { return Err(invalid("invalid support fixture hash")); }
        let mut bytes = [0u8;32];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            let nibble = |value: u8| if value <= b'9' { value-b'0' } else { value-b'a'+10 };
            bytes[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
        }
        Ok(bytes)
    }
    // Actual raw source files are independently hash-bound. The served parent
    // is .sub transformed; its raw source remains fixture input, not EOF proof.
    let read = |artifact: &Artifact| -> Result<Vec<u8>, CommandError> {
        let bytes = read_bounded(open_artifact(root, artifact)?, MAX_ARTIFACT_BYTES)?;
        if digest(&bytes) != artifact.sha256 { return Err(invalid("support source closure hash mismatch")); }
        Ok(bytes)
    };
    let parent = read(&contract.parent)?;
    let child = read(&contract.child)?;
    let mut imports: Vec<Vec<u8>> = Vec::new();
    imports.try_reserve_exact(6).map_err(|_| invalid("support source scratch allocation failed"))?;
    for source in &contract.imports { imports.push(read(source)?); }
    let imported: [&[u8];6] = std::array::from_fn(|index| imports[index].as_slice());
    let parent_url = url::Url::parse(&contract.parent_url).map_err(|_| invalid("invalid support parent URL"))?;
    let child_url = url::Url::parse(&contract.child_url).map_err(|_| invalid("invalid support child URL"))?;
    oryn_native::PinnedSupportDescriptor::from_fixture(
        fixed_hash(admitted_candidate)?, fixed_hash(&contract.descriptor)?,
        &parent_url, &child_url, &contract.revision, &parent, &child, imported)
        .ok_or_else(|| invalid("support fixture differs from pinned native closure"))
}

fn validate_issued_candidate(declared:&str,admitted:&str)->Result<(),CommandError>{
    if !hash_valid(admitted)||declared!=admitted{
        return Err(invalid("support fixture differs from admitted candidate"));
    }
    Ok(())
}
