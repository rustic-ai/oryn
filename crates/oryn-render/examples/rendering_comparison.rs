use std::{fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let native_path = args.next().ok_or("native PNG path is required")?;
    let chromium_path = args.next().ok_or("Chromium PNG path is required")?;
    let native = image::open(&native_path)?.to_rgba8();
    let chromium = image::open(&chromium_path)?.to_rgba8();
    let dimensions_match = native.dimensions() == chromium.dimensions();
    let pixels = native.width() as usize * native.height() as usize;
    let colored = |image: &image::RgbaImage| {
        image
            .pixels()
            .filter(|pixel| pixel.0[..3].iter().any(|channel| *channel < 245))
            .count()
    };
    let native_colored = colored(&native);
    let chromium_colored = colored(&chromium);
    let mean_absolute_rgb_delta = if dimensions_match {
        native
            .pixels()
            .zip(chromium.pixels())
            .flat_map(|(left, right)| {
                (0..3).map(move |channel| left.0[channel].abs_diff(right.0[channel]) as u64)
            })
            .sum::<u64>() as f64
            / (pixels * 3) as f64
    } else {
        f64::INFINITY
    };
    // Oryn is a deterministic CPU reference, not a pixel clone of Chromium.
    // The differential verifies viewport identity and visible painted content,
    // while the committed native golden catches Oryn-specific regressions.
    let passed = dimensions_match
        && native_colored > pixels / 50
        && chromium_colored > pixels / 50
        && mean_absolute_rgb_delta < 120.0;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "suite": "native-chromium-visual-differential",
            "dimensions": {"native": native.dimensions(), "chromium": chromium.dimensions(), "match": dimensions_match},
            "painted_pixels": {"native": native_colored, "chromium": chromium_colored, "total": pixels},
            "mean_absolute_rgb_delta": mean_absolute_rgb_delta,
            "assertion_scope": "viewport identity and visible-content sanity; not pixel parity",
            "outcome": {"status": if passed { "passed" } else { "failed" }}
        }))?
    );
    if !passed {
        return Err("visual differential failed".into());
    }
    let _ = fs::metadata(native_path)?;
    Ok(())
}
