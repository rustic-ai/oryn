use clap::Parser;
use oryn_native::{Browser, ContextOptions};

#[derive(Debug, Parser)]
#[command(name = "oryn-render", about = "Optional CPU renderer for Oryn pages")]
struct Args {
    #[arg(long, conflicts_with = "url")]
    html: Option<String>,
    #[arg(long, conflicts_with = "html")]
    url: Option<String>,
    #[arg(long)]
    png: Option<String>,
    #[arg(long)]
    pdf: Option<String>,
    #[arg(long)]
    allow_loopback: bool,
    #[arg(long)]
    scale: Option<f64>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.png.is_none() && args.pdf.is_none() {
        return Err("at least one of --png or --pdf is required".into());
    }
    let options = if args.allow_loopback {
        ContextOptions::loopback_test()
    } else {
        ContextOptions::default()
    };
    let context = Browser::new().new_context_with_options(options);
    let page = context.new_page();
    if let Some(path) = args.html {
        let html = std::fs::read_to_string(&path)?;
        page.load_html(format!("fixture://{path}"), html).await?;
    } else if let Some(url) = args.url {
        page.goto(url).await?;
    } else {
        page.load_html("about:blank", "<!doctype html>").await?;
    }
    let snapshot = page.paint_snapshot().await?;
    let options = oryn_render::RenderOptions {
        scale: args.scale.unwrap_or(snapshot.viewport.device_pixel_ratio),
        ..oryn_render::RenderOptions::default()
    };
    let mut diagnostics = Vec::new();
    if let Some(path) = args.png {
        let artifact = oryn_render::render_png(&snapshot, options)?;
        std::fs::write(&path, &artifact.bytes)?;
        diagnostics.extend(artifact.diagnostics);
    }
    if let Some(path) = args.pdf {
        let artifact = oryn_render::render_pdf(&snapshot, options)?;
        std::fs::write(&path, &artifact.bytes)?;
        diagnostics.extend(artifact.diagnostics);
    }
    diagnostics.sort();
    diagnostics.dedup();
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "status": "rendered",
            "page": snapshot.page,
            "revision": snapshot.revision,
            "diagnostics": diagnostics,
        }))?
    );
    Ok(())
}
