use oryn_parser::{parse, normalize, Script};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Deserialize)]
struct TestFile {
    #[serde(flatten)]
    groups: HashMap<String, TestGroup>,
}

#[derive(Debug, Deserialize)]
struct TestGroup {
    metadata: Metadata,
    vectors: Vec<TestVector>,
}

#[derive(Debug, Deserialize)]
struct Metadata {
    description: Option<String>,
    version: String,
}

#[derive(Debug, Deserialize)]
struct TestVector {
    id: String,
    raw: String,
    canonical: Option<String>,
    // ast: Option<serde_yaml::Value>, // We can try to match this against serialized AST, but structure might differ.
    // For now we check that:
    // 1. raw -> normalize -> matches canonical (if present)
    // 2. canonical -> parse -> succeeds
    // We can also verify parse error if "error" field exists
    error: Option<ErrorExpectation>,
}

#[derive(Debug, Deserialize)]
struct ErrorExpectation {
    phase: String, // parsing vs semantic
    // code: String,
}

#[test]
fn test_vectors_compliance() {
    let path = "../../grammar/oil-test-vectors.v1.8.1.yaml";
    let content = fs::read_to_string(path).unwrap_or_else(|_| panic!("Failed to read {}", path));
    
    let groups: HashMap<String, TestGroup> = serde_yaml::from_str(&content).expect("Failed to parse YAML");

    let mut passed = 0;
    let mut total = 0;
    let mut failures = Vec::new();
    
    for (group_name, group) in groups {
        println!("Testing group: {}", group_name);
        for vector in group.vectors {
            
            total += 1;
            // println!("  Vector: {}", vector.id);

            // 1. Normalization
            let normalized = normalize(&vector.raw);
            
            if let Some(expected_canonical) = &vector.canonical {
                 let normalized_trim = normalized.trim();
                 let expected_trim = expected_canonical.trim();
                 
                 if normalized_trim != expected_trim {
                     let msg = format!("Normalization mismatch for {}\n      Raw:    {:?}\n      Got:    {:?}\n      Expect: {:?}", 
                         vector.id, vector.raw, normalized_trim, expected_trim);
                     println!("    [FAIL] {}", msg);
                     failures.push(msg);
                     continue;
                 }
                 
                 // 2. Parse Canonical
                 // Note: We use expected_canonical for parsing test as per spec
                 match parse(expected_canonical) {
                     Ok(_) => {
                         if let Some(err_exp) = &vector.error {
                             if err_exp.phase == "parsing" {
                                  let msg = format!("Expected parsing error for {}, but succeeded", vector.id);
                                  println!("    [FAIL] {}", msg);
                                  failures.push(msg);
                             }
                         }
                     },
                     Err(e) => {
                         if vector.error.is_none() {
                             let msg = format!("Parse failed for {}: {}", vector.id, e);
                             println!("    [FAIL] {}", msg);
                             failures.push(msg);
                         }
                     }
                 }
            } else if let Some(err_exp) = &vector.error {
                 if err_exp.phase == "parsing" {
                     match parse(&normalized) {
                         Ok(_) => {
                             let msg = format!("Expected parsing error for {}, but succeeded", vector.id);
                             println!("    [FAIL] {}", msg);
                             failures.push(msg);
                         },
                         Err(_) => {} // OK
                     }
                 }
            }

            passed += 1;
        }
    }
    
    if !failures.is_empty() {
        println!("\nFailures:");
        for f in &failures {
            println!("- {}", f);
        }
        panic!("Failed {}/{} vectors", failures.len(), total);
    }
    
    println!("Passed {}/{} vectors", passed, total);
}
