//! Canvas compositing in premultiplied sRGB, with CSS blend functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Composite {
    #[default]
    SourceOver,
    SourceIn,
    SourceOut,
    SourceAtop,
    DestinationOver,
    DestinationIn,
    DestinationOut,
    DestinationAtop,
    Lighter,
    Copy,
    Xor,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl Composite {
    const ALL: &'static [(Self, &'static str)] = &[
        (Self::SourceOver, "source-over"),
        (Self::SourceIn, "source-in"),
        (Self::SourceOut, "source-out"),
        (Self::SourceAtop, "source-atop"),
        (Self::DestinationOver, "destination-over"),
        (Self::DestinationIn, "destination-in"),
        (Self::DestinationOut, "destination-out"),
        (Self::DestinationAtop, "destination-atop"),
        (Self::Lighter, "lighter"),
        (Self::Copy, "copy"),
        (Self::Xor, "xor"),
        (Self::Multiply, "multiply"),
        (Self::Screen, "screen"),
        (Self::Overlay, "overlay"),
        (Self::Darken, "darken"),
        (Self::Lighten, "lighten"),
        (Self::ColorDodge, "color-dodge"),
        (Self::ColorBurn, "color-burn"),
        (Self::HardLight, "hard-light"),
        (Self::SoftLight, "soft-light"),
        (Self::Difference, "difference"),
        (Self::Exclusion, "exclusion"),
        (Self::Hue, "hue"),
        (Self::Saturation, "saturation"),
        (Self::Color, "color"),
        (Self::Luminosity, "luminosity"),
    ];
    pub(super) fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(mode, _)| *mode)
    }
    pub(super) fn name(self) -> &'static str {
        Self::ALL.iter().find(|(mode, _)| *mode == self).unwrap().1
    }
    pub(super) fn affects_outside_source(self) -> bool {
        matches!(
            self,
            Self::SourceIn
                | Self::SourceOut
                | Self::DestinationIn
                | Self::DestinationAtop
                | Self::Copy
        )
    }

    // Source channels are premultiplied 0..255 values, source alpha is 0..1.
    pub(super) fn apply_clipped(self, target: &mut [u8], source: [f64; 4], coverage: f64) {
        if coverage <= 0.0 {
            return;
        }
        if coverage >= 1.0 {
            self.apply(target, source);
            return;
        }
        let original = [target[0], target[1], target[2], target[3]];
        self.apply(target, source);
        let before_alpha = original[3] as f64 / 255.0;
        let after_alpha = target[3] as f64 / 255.0;
        let alpha = before_alpha * (1.0 - coverage) + after_alpha * coverage;
        for i in 0..3 {
            let premultiplied = original[i] as f64 * before_alpha * (1.0 - coverage)
                + target[i] as f64 * after_alpha * coverage;
            target[i] = if alpha == 0.0 {
                0
            } else {
                (premultiplied / alpha).round().clamp(0.0, 255.0) as u8
            };
        }
        target[3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    pub(super) fn apply(self, target: &mut [u8], source: [f64; 4]) {
        let sa = source[3].clamp(0.0, 1.0);
        let da = target[3] as f64 / 255.0;
        let destination = [
            target[0] as f64 / 255.0,
            target[1] as f64 / 255.0,
            target[2] as f64 / 255.0,
        ];
        let src = if sa == 0.0 {
            [0.0; 3]
        } else {
            [
                source[0] / (255.0 * sa),
                source[1] / (255.0 * sa),
                source[2] / (255.0 * sa),
            ]
        };
        let factors = match self {
            Self::SourceOver => Some((1.0, 1.0 - sa)),
            Self::SourceIn => Some((da, 0.0)),
            Self::SourceOut => Some((1.0 - da, 0.0)),
            Self::SourceAtop => Some((da, 1.0 - sa)),
            Self::DestinationOver => Some((1.0 - da, 1.0)),
            Self::DestinationIn => Some((0.0, sa)),
            Self::DestinationOut => Some((0.0, 1.0 - sa)),
            Self::DestinationAtop => Some((1.0 - da, sa)),
            Self::Lighter => Some((1.0, 1.0)),
            Self::Copy => Some((1.0, 0.0)),
            Self::Xor => Some((1.0 - da, 1.0 - sa)),
            _ => None,
        };
        let (alpha, premultiplied) = if let Some((fs, fd)) = factors {
            let alpha = (sa * fs + da * fd).min(1.0);
            (
                alpha,
                std::array::from_fn::<_, 3, _>(|i| {
                    (src[i] * sa * fs + destination[i] * da * fd).min(1.0)
                }),
            )
        } else {
            let blend = self.blend(destination, src);
            (
                sa + da - sa * da,
                std::array::from_fn(|i| {
                    src[i] * sa * (1.0 - da) + destination[i] * da * (1.0 - sa) + blend[i] * sa * da
                }),
            )
        };
        let alpha_byte = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        for i in 0..3 {
            target[i] = if alpha_byte == 0 {
                0
            } else {
                (premultiplied[i] / alpha * 255.0).round().clamp(0.0, 255.0) as u8
            };
        }
        target[3] = alpha_byte;
    }

    fn blend(self, backdrop: [f64; 3], source: [f64; 3]) -> [f64; 3] {
        match self {
            Self::Hue => set_lum(set_sat(source, sat(backdrop)), lum(backdrop)),
            Self::Saturation => set_lum(set_sat(backdrop, sat(source)), lum(backdrop)),
            Self::Color => set_lum(source, lum(backdrop)),
            Self::Luminosity => set_lum(backdrop, lum(source)),
            _ => std::array::from_fn(|i| {
                let b = backdrop[i];
                let s = source[i];
                match self {
                    Self::Multiply => b * s,
                    Self::Screen => b + s - b * s,
                    Self::Overlay => hard_light(s, b),
                    Self::Darken => b.min(s),
                    Self::Lighten => b.max(s),
                    Self::ColorDodge => {
                        if b == 0.0 {
                            0.0
                        } else if s == 1.0 {
                            1.0
                        } else {
                            (b / (1.0 - s)).min(1.0)
                        }
                    }
                    Self::ColorBurn => {
                        if b == 1.0 {
                            1.0
                        } else if s == 0.0 {
                            0.0
                        } else {
                            1.0 - ((1.0 - b) / s).min(1.0)
                        }
                    }
                    Self::HardLight => hard_light(b, s),
                    Self::SoftLight => {
                        if s <= 0.5 {
                            b - (1.0 - 2.0 * s) * b * (1.0 - b)
                        } else {
                            let d = if b <= 0.25 {
                                ((16.0 * b - 12.0) * b + 4.0) * b
                            } else {
                                b.sqrt()
                            };
                            b + (2.0 * s - 1.0) * (d - b)
                        }
                    }
                    Self::Difference => (b - s).abs(),
                    Self::Exclusion => b + s - 2.0 * b * s,
                    _ => s,
                }
            }),
        }
    }
}

fn hard_light(b: f64, s: f64) -> f64 {
    if s <= 0.5 {
        2.0 * b * s
    } else {
        1.0 - 2.0 * (1.0 - b) * (1.0 - s)
    }
}
fn lum(c: [f64; 3]) -> f64 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}
fn sat(c: [f64; 3]) -> f64 {
    c.into_iter().fold(f64::NEG_INFINITY, f64::max) - c.into_iter().fold(f64::INFINITY, f64::min)
}
fn set_lum(mut c: [f64; 3], value: f64) -> [f64; 3] {
    let delta = value - lum(c);
    for channel in &mut c {
        *channel += delta;
    }
    let l = lum(c);
    let min = c.into_iter().fold(f64::INFINITY, f64::min);
    let max = c.into_iter().fold(f64::NEG_INFINITY, f64::max);
    if min < 0.0 {
        for channel in &mut c {
            *channel = l + (*channel - l) * l / (l - min);
        }
    }
    if max > 1.0 {
        for channel in &mut c {
            *channel = l + (*channel - l) * (1.0 - l) / (max - l);
        }
    }
    c
}
fn set_sat(mut c: [f64; 3], value: f64) -> [f64; 3] {
    let mut order = [0, 1, 2];
    order.sort_by(|a, b| c[*a].total_cmp(&c[*b]));
    let [min, mid, max] = order;
    if c[max] > c[min] {
        c[mid] = (c[mid] - c[min]) * value / (c[max] - c[min]);
        c[max] = value;
    } else {
        c[mid] = 0.0;
        c[max] = 0.0;
    }
    c[min] = 0.0;
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn porter_duff_opaque_red_over_blue() {
        for (mode, expected) in [
            (Composite::SourceOver, [255, 0, 0, 255]),
            (Composite::SourceIn, [255, 0, 0, 255]),
            (Composite::SourceOut, [0, 0, 0, 0]),
            (Composite::SourceAtop, [255, 0, 0, 255]),
            (Composite::DestinationOver, [0, 0, 255, 255]),
            (Composite::DestinationIn, [0, 0, 255, 255]),
            (Composite::DestinationOut, [0, 0, 0, 0]),
            (Composite::DestinationAtop, [0, 0, 255, 255]),
            (Composite::Lighter, [255, 0, 255, 255]),
            (Composite::Copy, [255, 0, 0, 255]),
            (Composite::Xor, [0, 0, 0, 0]),
            (Composite::Multiply, [0, 0, 0, 255]),
            (Composite::Screen, [255, 0, 255, 255]),
            (Composite::Difference, [255, 0, 255, 255]),
        ] {
            let mut target = [0, 0, 255, 255];
            mode.apply(&mut target, [255.0, 0.0, 0.0, 1.0]);
            assert_eq!(target, expected, "{}", mode.name());
        }
    }
    #[test]
    fn partial_alpha_and_transparent_source() {
        let mut target = [0, 0, 255, 255];
        Composite::SourceOver.apply(&mut target, [127.5, 0.0, 0.0, 0.5]);
        assert_eq!(target, [128, 0, 128, 255]);
        for &(mode, name) in Composite::ALL {
            assert_eq!(Composite::parse(name), Some(mode));
            let mut target = [23, 56, 89, 128];
            mode.apply(&mut target, [0.0; 4]);
            assert_eq!(
                target,
                if mode.affects_outside_source() {
                    [0, 0, 0, 0]
                } else {
                    [23, 56, 89, 128]
                },
                "{name}"
            );
        }
        assert_eq!(Composite::parse("SOURCE-OVER"), None);
    }
    #[test]
    fn nonseparable_blends_preserve_requested_luminance_and_saturation() {
        let b = [0.2, 0.4, 0.7];
        let s = [0.7, 0.3, 0.4];
        for mode in [Composite::Hue, Composite::Saturation, Composite::Color] {
            assert!((lum(mode.blend(b, s)) - lum(b)).abs() < 1e-12);
        }
        assert!((lum(Composite::Luminosity.blend(b, s)) - lum(s)).abs() < 1e-12);
        assert!((sat(Composite::Saturation.blend(b, s)) - sat(s)).abs() < 1e-12);
    }
}
