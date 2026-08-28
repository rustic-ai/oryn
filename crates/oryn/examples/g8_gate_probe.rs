use std::{fs, path::PathBuf};

use oryn_common::{v3::SupportLevel, v4::ViewportState};
use oryn_native::{Browser, ContextOptions};
use oryn_render::{PaintCommand, RenderOptions};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("artifacts/g8-v1/probe"));
    fs::create_dir_all(&output_dir)?;
    let context = Browser::new().new_context_with_options(ContextOptions::default().with_viewport(
        ViewportState {
            width: 320.0,
            height: 200.0,
            device_pixel_ratio: 1.0,
        },
    ));
    let page = context.new_page();
    page.load_html(
        "https://g8.example.test/probe",
        r#"<!doctype html><style>
          html,body{margin:0;background-color:#fafafa;color:#102030}
          #card{width:220px;height:72px;padding:8px;border:3px solid #204080;background-color:#dceeff;font-size:18px}
          img{display:block;width:16px;height:16px}
        </style><div id=card>Oryn CPU paint
        <img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII="></div>"#,
    )
    .await?;
    let core_capabilities = page.capabilities().await?;
    let snapshot = page.paint_snapshot().await?;
    let options = RenderOptions::default();
    let scene = oryn_render::build_display_list(&snapshot, options)?;
    let png = oryn_render::render_png(&snapshot, options)?;
    let second_png = oryn_render::render_png(&snapshot, options)?;
    let pdf = oryn_render::render_pdf(&snapshot, options)?;
    fs::write(output_dir.join("g8-probe.png"), &png.bytes)?;
    fs::write(output_dir.join("g8-probe.pdf"), &pdf.bytes)?;
    fs::write(
        output_dir.join("paint-snapshot.json"),
        serde_json::to_vec_pretty(&snapshot)?,
    )?;
    let has = |predicate: fn(&PaintCommand) -> bool| scene.commands.iter().any(predicate);
    let criteria = serde_json::json!({
        "paint_snapshot_roundtrip": snapshot.contract_version == oryn_common::v4::CONTRACT_VERSION,
        "document_order_scene": scene.commands.len() >= 4,
        "background_paint": has(|command| matches!(command, PaintCommand::FillRect { .. })),
        "border_paint": has(|command| matches!(command, PaintCommand::Border { .. })),
        "text_paint": has(|command| matches!(command, PaintCommand::Text { .. })),
        "image_paint": has(|command| matches!(command, PaintCommand::Image { .. })),
        "deterministic_png": png.bytes == second_png.bytes,
        "valid_png": png.bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "valid_pdf": pdf.bytes.starts_with(b"%PDF-1.4") && pdf.bytes.ends_with(b"%%EOF\n"),
        "dimensions": png.width == 320 && png.height == 200,
        "core_boundary": core_capabilities.iter().any(|capability| capability.capability == "paint" && capability.support == SupportLevel::Unsupported),
        "snapshot_capability": core_capabilities.iter().any(|capability| capability.capability == "paint.snapshot" && capability.support == SupportLevel::Supported),
    });
    let passed = criteria
        .as_object()
        .is_some_and(|values| values.values().all(|value| value == true));
    let result = serde_json::json!({
        "schema_version": 1,
        "suite": "oryn-g8-cpu-reference-probe",
        "renderer_capabilities": oryn_render::capabilities(),
        "snapshot": {
            "nodes": snapshot.nodes.len(),
            "images": snapshot.images.len(),
            "diagnostics": snapshot.diagnostics,
        },
        "scene": {"commands": scene.commands.len(), "diagnostics": scene.diagnostics},
        "artifacts": {
            "png": {"path": "g8-probe.png", "bytes": png.bytes.len(), "width": png.width, "height": png.height},
            "pdf": {"path": "g8-probe.pdf", "bytes": pdf.bytes.len(), "width": pdf.width, "height": pdf.height}
        },
        "criteria": criteria,
        "outcome": {"status": if passed { "passed" } else { "failed" }}
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    if !passed {
        return Err("G8 probe criteria failed".into());
    }
    Ok(())
}
