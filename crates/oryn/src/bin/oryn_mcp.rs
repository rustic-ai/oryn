//! MCP stdio entry point. Protocol messages are one JSON object per line;
//! diagnostics go to stderr so stdout remains a clean JSON-RPC channel.

use clap::Parser;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Parser)]
#[command(name = "oryn-mcp")]
struct Args {
    /// Read origin-scoped secret records from this inherited descriptor.
    #[arg(long, value_name = "FD")]
    secret_fd: Option<i32>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut server = oryn::mcp::McpServer::with_secret_fd(args.secret_fd);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::BufWriter::new(tokio::io::stdout());
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {"code": -32700, "message": error.to_string()}
                });
                stdout
                    .write_all(format!("{}\n", serde_json::to_string(&response)?).as_bytes())
                    .await?;
                stdout.flush().await?;
                continue;
            }
        };
        if let Some(response) = server.handle(request).await {
            stdout
                .write_all(format!("{}\n", serde_json::to_string(&response)?).as_bytes())
                .await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}
