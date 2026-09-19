//! Exercise genuinely first-use allocator initialization in a fresh process,
//! independent of the test harness's ordering and existing V8 tests.
#[test]
fn concurrent_first_isolates_initialize_the_shared_allocator_once() {
    const CHILD: &str = "ORYN_TEST_CONCURRENT_FIRST_V8_ALLOCATOR";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "v8_bindings::v8_initialization_tests::concurrent_first_isolates_initialize_the_shared_allocator_once",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fresh V8 allocator race failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                super::tests::initialize_v8();
                for _ in 0..4 {
                    let mut isolate = v8::Isolate::new(Default::default());
                    v8::scope!(let scope, &mut isolate);
                    let context = v8::Context::new(scope, Default::default());
                    let scope = &mut v8::ContextScope::new(scope, context);
                    let source =
                        v8::String::new(scope, "const b=new Uint8Array(4096);b[4095]=123;b[4095]")
                            .unwrap();
                    let value = v8::Script::compile(scope, source, None)
                        .unwrap()
                        .run(scope)
                        .unwrap();
                    assert_eq!(value.integer_value(scope), Some(123));
                    let buffer = v8::ArrayBuffer::new(scope, 4096);
                    assert_eq!(buffer.byte_length(), 4096);
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
