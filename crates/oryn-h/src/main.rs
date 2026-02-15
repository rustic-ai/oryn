use clap::Parser as ClapParser;
use oryn_engine::backend::Backend;
use oryn_engine::cli::{self, FileErrorMode, FileOptions, OutputHandlers, ReplOptions};
use oryn_engine::executor::CommandExecutor;
use oryn_h::backend::HeadlessBackend;

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Script file to execute
    #[arg(long)]
    file: Option<String>,

    /// Launch browser in visible mode (not headless)
    #[arg(long)]
    visible: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    println!("Starting Oryn Headless Backend...");

    let mut backend = HeadlessBackend::new_with_visibility(args.visible);
    backend.launch().await?;
    let mut executor = CommandExecutor::new();
    let output = OutputHandlers {
        out: |msg| println!("{}", msg),
        err: |msg| println!("{}", msg),
    };
    let repl_options = ReplOptions {
        banner_lines: &["Backend launched. Enter commands (e.g., 'goto google.com', 'scan')."],
        prompt: "> ",
        exit_commands: &["exit", "quit"],
        handle_ctrl_c: false,
        ctrl_c_message: None,
    };

    if let Some(file_path) = args.file {
        cli::run_file(
            &mut backend,
            &mut executor,
            output,
            &file_path,
            FileOptions {
                stop_on_error: true,
                error_mode: FileErrorMode::Plain,
            },
        )
        .await?;
    } else {
        cli::run_repl(&mut backend, &mut executor, output, repl_options).await?;
    }

    backend.close().await?;
    Ok(())
}
