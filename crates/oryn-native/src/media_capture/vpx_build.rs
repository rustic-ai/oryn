// Executed only by Cargo's build phase; source-generation never invokes this.
pub fn build() -> std::path::PathBuf {
    use std::{env, fs, path::PathBuf, process::Command};
    let source = PathBuf::from("../../vendor/libvpx")
        .canonicalize()
        .expect("pinned libvpx source");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("libvpx");
    fs::create_dir_all(&output).expect("libvpx output");
    let compiler = cc::Build::new().get_compiler();
    let cxx_compiler = cc::Build::new().cpp(true).get_compiler();
    let archiver = cc::Build::new().get_archiver();
    let mut configure = Command::new("sh");
    configure
        .arg(source.join("configure"))
        .current_dir(&output)
        .args([
            "--target=generic-gnu",
            "--disable-vp9",
            "--disable-vp8-encoder",
            "--disable-examples",
            "--disable-tools",
            "--disable-docs",
            "--disable-unit-tests",
            "--disable-install-docs",
            "--enable-pic",
            "--disable-shared",
            "--enable-static",
            "--disable-runtime-cpu-detect",
        ]);
    // Upstream configure and make expand tool commands by shell word splitting.
    // Preserve Cargo's executable, arguments and environment in native wrappers
    // for both compilation and archiving (including SDK paths containing spaces).
    fn quote(value: &std::ffi::OsStr) -> String {
        format!(
            "'{}'",
            value
                .to_str()
                .expect("native tool argument UTF-8")
                .replace('\'', "'\\''")
        )
    }
    fn wrapper(output: &std::path::Path, name: &str, tool: &Command) {
        let mut script = String::from("#!/bin/sh\nexec env");
        // env options precede assignments; explicit removals must not be treated as
        // arguments to the selected native tool.
        for (name, value) in tool.get_envs() {
            if value.is_none() {
                script.push_str(" -u ");
                script.push_str(&quote(name));
            }
        }
        for (name, value) in tool.get_envs() {
            if let Some(value) = value {
                let mut assignment = name.to_os_string();
                assignment.push("=");
                assignment.push(value);
                script.push(' ');
                script.push_str(&quote(&assignment));
            }
        }
        script.push(' ');
        script.push_str(&quote(tool.get_program()));
        for argument in tool.get_args() {
            script.push(' ');
            script.push_str(&quote(argument));
        }
        script.push_str(" \"$@\"\n");
        let path = output.join(name);
        fs::write(&path, script).expect("libvpx native tool wrapper");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("libvpx tool wrapper mode");
        }
    }
    wrapper(&output, "oryn-vpx-cc", &compiler.to_command());
    // libvpx selects CXX as its linker when webm_io is enabled. Preserve the
    // Cargo-selected C++ driver and SDK flags just as for C compilation.
    wrapper(&output, "oryn-vpx-cxx", &cxx_compiler.to_command());
    wrapper(&output, "oryn-vpx-ar", &archiver);
    configure
        .env("CC", "./oryn-vpx-cc")
        .env("CXX", "./oryn-vpx-cxx")
        .env("LD", "./oryn-vpx-cxx")
        .env("CFLAGS", "")
        .env("CXXFLAGS", "")
        .env("AR", "./oryn-vpx-ar");
    assert!(
        configure.status().expect("libvpx configure").success(),
        "libvpx decoder configuration failed"
    );
    assert!(
        Command::new("make")
            .current_dir(&output)
            .arg("-j")
            .arg(env::var("NUM_JOBS").unwrap_or_else(|_| "1".into()))
            .arg("libvpx.a")
            .status()
            .expect("libvpx make")
            .success(),
        "libvpx decoder compilation failed"
    );
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=vpx");
    source
}
