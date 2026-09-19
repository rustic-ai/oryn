//! Replace the launcher with the explicitly selected runtime so that worker
//! discovery and runtime identity belong to that artifact, and signals reach it.
use clap::Parser;
#[derive(Parser)]
#[command(name = "oryn-webdriver")]
struct LaunchArgs {
    #[arg(long)]
    browser_binary: std::path::PathBuf,
    #[command(flatten)]
    service: oryn::webdriver::cli::ServeArgs,
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = LaunchArgs::parse();
    if !args.browser_binary.is_absolute() {
        return Err("browser binary must be an absolute path".into());
    }
    let binary = args.browser_binary.canonicalize()?;
    if binary == std::env::current_exe()?.canonicalize()? {
        return Err("driver cannot launch itself as browser".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new(binary)
            .args(args.service.forwarded_args())
            .exec();
        Err(error.into())
    }
    #[cfg(not(unix))]
    {
        Err("native WebDriver launcher requires Unix process replacement".into())
    }
}
