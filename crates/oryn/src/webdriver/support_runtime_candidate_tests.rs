// Install within fixture.rs tests. Exact runtime digest mismatch is rejected.
#[cfg(unix)]
#[test]
fn support_candidate_wrong_runtime_rejects_before_fixture_issuance(){
    // Native fixture loader owns regular-file candidate input and actual
    // executable comparison, not a fixture selfdeclared success boolean.
    let directory=Directory::new();
    let (manifest,expected)=directory.write_contract(&serde_json::json!({"candidate":"test-input"}));
    // Exact small manifest check succeeds, forcing actual executable mismatch.
    assert!(verify_support_candidate_launch(&manifest,&expected,&"1".repeat(64),
        &"0".repeat(64)).is_err());
}

#[test]
fn support_descriptor_candidate_substitution_rejected(){
    // Actual issuer compares independently admitted candidate before any raw
    // source closure is issued; the Python canonical producer has parallel
    // original-run identity controls, not an author list registration API.
    let candidate="1".repeat(64);
    let substituted="2".repeat(64);
    assert!(validate_issued_candidate(&candidate,&candidate).is_ok());
    assert!(validate_issued_candidate(&substituted,&candidate).is_err());
    assert!(validate_issued_candidate(&candidate,"").is_err());
}
