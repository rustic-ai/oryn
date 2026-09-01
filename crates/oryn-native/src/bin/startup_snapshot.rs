use std::{env, fs, process::ExitCode, sync::Once};

static INITIALIZE: Once = Once::new();

fn main() -> ExitCode {
    let Some(output) = env::args().nth(1) else {
        eprintln!("usage: oryn-startup-snapshot <output.blob>");
        return ExitCode::FAILURE;
    };
    INITIALIZE.call_once(|| {
        oryn_native::initialize_v8_icu_data();
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
    let mut isolate =
        v8::Isolate::snapshot_creator(Some(oryn_native::host::startup_external_references()), None);
    {
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        oryn_native::host::install_startup_snapshot_globals(scope, context);
        let source = v8::String::new(
            scope,
            "Object.defineProperties(globalThis,{__oryn_snapshot_version:{value:2},__oryn_snapshot_binding_abi:{value:1}});",
        )
        .expect("snapshot source");
        v8::Script::compile(scope, source, None)
            .and_then(|script| script.run(scope))
            .expect("snapshot initialization");
        scope.set_default_context(context);
    }
    let blob = isolate
        .create_blob(v8::FunctionCodeHandling::Keep)
        .expect("startup snapshot blob");
    fs::write(&output, &*blob).expect("write startup snapshot");
    println!("{output}");
    ExitCode::SUCCESS
}
