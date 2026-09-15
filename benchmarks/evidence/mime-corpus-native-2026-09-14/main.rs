#[allow(dead_code)]
#[path = "/Users/rohit/Work/oryn2/oryn-roadmap-next/crates/oryn-js/src/mime_type.rs"]
mod mime_type;
use mime_type::MimeType;

fn main() {
    let mut failures = 0;
    for path in std::env::args().skip(1) {
        let cases: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut total = 0;
        let mut failed = 0;
        for case in cases.as_array().unwrap().iter().filter(|case| case.is_object()) {
            total += 1;
            let input = case["input"].as_str().unwrap();
            let expected = case["output"].as_str();
            let actual = MimeType::parse(input).map(|mime| mime.serialize());
            if actual.as_deref() != expected {
                failed += 1;
                eprintln!("{}", serde_json::json!({"input":input,"expected":expected,"actual":actual}));
            }
        }
        println!("{}", serde_json::json!({"path":path,"total":total,"passed":total-failed,"failed":failed,"qualification":"direct native MIME parser corpus, not network WPT execution"}));
        failures += failed;
    }
    if failures != 0 { std::process::exit(1); }
}
