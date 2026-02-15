use clap::Parser as ClapParser;
use oryn_engine::backend::Backend;
use oryn_engine::cli::{self, OutputHandlers, ReplOptions};
use oryn_engine::executor::CommandExecutor;
use oryn_r::backend::RemoteBackend;

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = 9001)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    println!("Starting Oryn Remote Backend on port {}...", args.port);
    println!(
        "Please connect the browser extension to ws://localhost:{}",
        args.port
    );

    let mut backend = RemoteBackend::new(args.port);
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

    cli::run_repl(&mut backend, &mut executor, output, repl_options).await?;

    backend.close().await?;
    Ok(())
}
