//! Viewport screenshots through the native paint boundary.
use super::CommandError;
use oryn_native::PageHandle;

#[cfg(feature = "render")]
pub async fn capture(page: &PageHandle) -> Result<String, CommandError> {
    use base64::Engine as _;
    let snapshot = page.paint_snapshot().await.map_err(|error| CommandError {
        code: "unable to capture screen",
        message: error.to_string(),
    })?;
    if snapshot.viewport.width <= 0. || snapshot.viewport.height <= 0. {
        return Err(CommandError {
            code: "unable to capture screen",
            message: "viewport has no capture area".into(),
        });
    }
    let options = oryn_render::RenderOptions {
        scale: snapshot.viewport.device_pixel_ratio,
        ..Default::default()
    };
    let artifact = tokio::task::spawn_blocking(move || oryn_render::render_png(&snapshot, options))
        .await
        .map_err(|_| CommandError {
            code: "unable to capture screen",
            message: "native renderer task failed".into(),
        })?
        .map_err(|error| CommandError {
            code: "unable to capture screen",
            message: error.to_string(),
        })?;
    if !artifact.diagnostics.is_empty() {
        return Err(CommandError {
            code: "unable to capture screen",
            message: format!(
                "native rendering incomplete: {}",
                artifact.diagnostics.join("; ")
            ),
        });
    }
    if artifact.bytes.len() > 32 * 1024 * 1024 {
        return Err(CommandError {
            code: "unable to capture screen",
            message: "encoded screenshot exceeds output limit".into(),
        });
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(artifact.bytes))
}

#[cfg(not(feature = "render"))]
pub async fn capture(_page: &PageHandle) -> Result<String, CommandError> {
    Err(CommandError {
        code: "unsupported operation",
        message: "native screenshot requires the render feature".into(),
    })
}

#[cfg(all(test, feature = "render"))]
mod tests {
    use super::*;
    use base64::Engine as _;
    #[tokio::test]
    async fn screenshot_returns_png_for_native_viewport_and_rejects_retired_page() {
        let browser = oryn_native::Browser::new();
        let context = browser.new_audited_context_with_isolated_auxiliaries(
            oryn_native::ContextOptions::loopback_test(),
        );
        let page = context.new_page();
        page.load_html(
            "https://capture.test/",
            "<style>html,body{margin:0;background:red}</style>",
        )
        .await
        .unwrap();
        for scale in [1., 2.] {
            page.set_viewport(oryn_common::v4::ViewportState {
                width: 24.,
                height: 16.,
                device_pixel_ratio: scale,
            })
            .await
            .unwrap();
            let encoded = capture(&page).await.unwrap();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap();
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
            assert_eq!(
                u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
                (24. * scale) as u32
            );
            assert_eq!(
                u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
                (16. * scale) as u32
            );
        }
        context.close_page(page.page_id()).await.unwrap();
        context.retire_workers().await.unwrap();
        assert!(capture(&page).await.is_err());
    }
}
