//! CSS box-shadow masks, sampled in the box's own plane before projection.
use super::*;
pub use oryn_common::box_shadow::BoxShadow;
pub(super) use oryn_common::box_shadow::{bounds, parse};
fn spread_rect(rect: CssRect, spread: f64, offset: [f64; 2]) -> CssRect {
    CssRect {
        x: rect.x - spread + offset[0],
        y: rect.y - spread + offset[1],
        width: (rect.width + 2. * spread).max(0.),
        height: (rect.height + 2. * spread).max(0.),
    }
}
fn spread_radii(mut radii: [f64; 8], spread: f64, rect: CssRect) -> [f64; 8] {
    for radius in &mut radii {
        if *radius > 0. {
            let adjustment = if spread > 0. && *radius < spread {
                let ratio = *radius / spread;
                spread * (1. + (ratio - 1.).powi(3))
            } else {
                spread
            };
            *radius = (*radius + adjustment).max(0.);
        }
    }
    let factor = 1f64
        .min(rect.width / (radii[0] + radii[2]).max(1e-12))
        .min(rect.width / (radii[6] + radii[4]).max(1e-12))
        .min(rect.height / (radii[1] + radii[7]).max(1e-12))
        .min(rect.height / (radii[3] + radii[5]).max(1e-12));
    radii.map(|radius| radius * factor)
}

pub(super) struct ShadowBitmap {
    pub rect: CssRect,
    pub width: usize,
    pub height: usize,
    pub alpha: Vec<u8>,
    scale: f64,
}
impl ShadowBitmap {
    pub(super) fn sample(&self, x: f64, y: f64) -> u8 {
        let x = (x - self.rect.x) * self.scale - 0.5;
        let y = (y - self.rect.y) * self.scale - 0.5;
        let ix = x.floor() as i64;
        let iy = y.floor() as i64;
        let mut alpha = 0.;
        for oy in 0..2 {
            for ox in 0..2 {
                let px = ix + ox;
                let py = iy + oy;
                if px >= 0 && py >= 0 && (px as usize) < self.width && (py as usize) < self.height {
                    let weight = (if ox == 0 {
                        1. - (x - ix as f64)
                    } else {
                        x - ix as f64
                    }) * (if oy == 0 {
                        1. - (y - iy as f64)
                    } else {
                        y - iy as f64
                    });
                    alpha += f64::from(self.alpha[py as usize * self.width + px as usize]) * weight;
                }
            }
        }
        alpha.round().clamp(0., 255.) as u8
    }
}
pub(super) fn raster(
    rect: CssRect,
    radii: [f64; 8],
    shadow: &BoxShadow,
    scale: f64,
) -> Result<ShadowBitmap, RenderError> {
    if !scale.is_finite()
        || scale <= 0.
        || [
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            shadow.blur,
            shadow.spread,
            shadow.offset[0],
            shadow.offset[1],
        ]
        .iter()
        .any(|v| !v.is_finite())
        || shadow.blur < 0.
    {
        return Err(RenderError::Encode("invalid box-shadow geometry".into()));
    }
    let mut area = if shadow.inset {
        let extent = shadow.blur * 2.;
        CssRect {
            x: rect.x - extent,
            y: rect.y - extent,
            width: rect.width + extent * 2.,
            height: rect.height + extent * 2.,
        }
    } else {
        bounds(rect, shadow)
    };
    let x = (area.x * scale).floor() / scale;
    let y = (area.y * scale).floor() / scale;
    area.width = ((area.right() * scale).ceil() / scale - x).max(1. / scale);
    area.height = ((area.bottom() * scale).ceil() / scale - y).max(1. / scale);
    area.x = x;
    area.y = y;
    let width = (area.width * scale).round();
    let height = (area.height * scale).round();
    let sigma = shadow.blur * scale / 2.;
    let radius = (sigma * 4.).ceil();
    if !width.is_finite()
        || !height.is_finite()
        || width * height > 16. * 1024. * 1024.
        || radius > 4096.
        || width * height * (2. * radius + 1.) > 256. * 1024. * 1024.
    {
        return Err(RenderError::Encode(
            "box-shadow surface or convolution exceeds bounds".into(),
        ));
    }
    let width = width as usize;
    let height = height as usize;
    let radius = radius as usize;
    let silhouette = spread_rect(
        rect,
        if shadow.inset {
            -shadow.spread
        } else {
            shadow.spread
        },
        shadow.offset,
    );
    let radii = spread_radii(
        radii,
        if shadow.inset {
            -shadow.spread
        } else {
            shadow.spread
        },
        silhouette,
    );
    let mut mask = vec![0f32; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut coverage = 0.;
            for sy in 0..4 {
                for sx in 0..4 {
                    let inside = silhouette.width > 0.
                        && silhouette.height > 0.
                        && borders::inside(
                            silhouette,
                            radii,
                            area.x + (x as f64 + (sx as f64 + 0.5) / 4.) / scale,
                            area.y + (y as f64 + (sy as f64 + 0.5) / 4.) / scale,
                        );
                    if inside != shadow.inset {
                        coverage += 1.;
                    }
                }
            }
            mask[y * width + x] = coverage / 16.;
        }
    }
    if radius > 0 {
        let mut kernel = (0..=radius * 2)
            .map(|i| (-0.5 * ((i as f64 - radius as f64) / sigma).powi(2)).exp() as f32)
            .collect::<Vec<_>>();
        let sum = kernel.iter().sum::<f32>();
        for weight in &mut kernel {
            *weight /= sum;
        }
        let outside = if shadow.inset { 1. } else { 0. };
        let mut temp = vec![0f32; mask.len()];
        for y in 0..height {
            for x in 0..width {
                temp[y * width + x] = kernel
                    .iter()
                    .enumerate()
                    .map(|(i, w)| {
                        let px = x as isize + i as isize - radius as isize;
                        w * if px < 0 || px >= width as isize {
                            outside
                        } else {
                            mask[y * width + px as usize]
                        }
                    })
                    .sum();
            }
        }
        for y in 0..height {
            for x in 0..width {
                mask[y * width + x] = kernel
                    .iter()
                    .enumerate()
                    .map(|(i, w)| {
                        let py = y as isize + i as isize - radius as isize;
                        w * if py < 0 || py >= height as isize {
                            outside
                        } else {
                            temp[py as usize * width + x]
                        }
                    })
                    .sum();
            }
        }
    }
    Ok(ShadowBitmap {
        rect: area,
        width,
        height,
        alpha: mask
            .into_iter()
            .map(|v| (v * 255.).round().clamp(0., 255.) as u8)
            .collect(),
        scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_multiple_shadow_list_preserves_front_to_back_and_units() {
        let values = parse(
            "purple 0em -1em, blue 1em 0em, fuchsia 0em 1em, orange -1em 0em",
            "black",
            50.,
        )
        .unwrap();
        assert_eq!(values.len(), 4);
        assert_eq!(values[0].offset, [0., -50.]);
        assert_eq!(values[3].offset, [-50., 0.]);
        assert_eq!(values[0].color, [128, 0, 128, 255]);
        assert!(parse("0px 0px -2px red", "black", 16.).is_none());
    }
    #[test]
    fn inset_and_outset_clip_to_opposite_sides_of_the_box() {
        let snapshot = crate::tests::empty_snapshot();
        let rect = CssRect {
            x: 10.,
            y: 8.,
            width: 20.,
            height: 16.,
        };
        for inset in [false, true] {
            let shadow = BoxShadow {
                offset: [10., 0.],
                blur: 0.,
                spread: 0.,
                color: [255, 0, 0, 255],
                inset,
            };
            let scene = DisplayList {
                contract_version: RENDER_CONTRACT_VERSION,
                width: 64,
                height: 32,
                commands: vec![PaintCommand::BoxShadow {
                    rect,
                    shadow,
                    radii: [0.; 8],
                }],
                diagnostics: Vec::new(),
            };
            let rgba = rasterize(
                &snapshot,
                &scene,
                RenderOptions {
                    background: [0; 4],
                    ..Default::default()
                },
            )
            .unwrap();
            let alpha = |x: usize| rgba[(16 * 64 + x) * 4 + 3];
            if inset {
                assert_eq!(alpha(15), 255);
                assert_eq!(alpha(25), 0);
                assert_eq!(alpha(35), 0);
            } else {
                assert_eq!(alpha(15), 0);
                assert_eq!(alpha(25), 0);
                assert_eq!(alpha(35), 255);
            }
        }
    }
    #[test]
    fn gaussian_blur_decays_smoothly_and_large_convolution_is_bounded() {
        let rect = CssRect {
            x: 20.,
            y: 20.,
            width: 20.,
            height: 20.,
        };
        let mut shadow = BoxShadow {
            offset: [0., 0.],
            blur: 4.,
            spread: 0.,
            color: [0, 0, 0, 255],
            inset: false,
        };
        let mask = raster(rect, [0.; 8], &shadow, 1.).unwrap();
        assert!((110..=145).contains(&mask.sample(40., 30.)));
        assert!(mask.sample(41., 30.) > mask.sample(44., 30.));
        assert!(mask.sample(30., 30.) > 250);
        shadow.blur = 1_000_000.;
        assert!(raster(rect, [0.; 8], &shadow, 1.).is_err());
    }
}
