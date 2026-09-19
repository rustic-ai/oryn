//! Deterministic native control theme. Values come from the native control
//! snapshot, while author appearance:none suppresses the themed decoration.
use crate::{PaintCommand, RenderError};
use oryn_common::v4::*;
fn fill(rect: CssRect, color: [u8; 4], radius: f64) -> PaintCommand {
    PaintCommand::FillRoundedRect {
        rect,
        color,
        radii: [radius; 8],
    }
}
fn inset(rect: CssRect, n: f64) -> CssRect {
    CssRect {
        x: rect.x + n,
        y: rect.y + n,
        width: (rect.width - 2. * n).max(0.),
        height: (rect.height - 2. * n).max(0.),
    }
}
fn segment(x: f64, y: f64, length: f64, width: f64, angle: f64, color: [u8; 4]) -> PaintCommand {
    let (c, s) = (angle.cos(), angle.sin());
    PaintCommand::Projected {
        matrix: [c, s, 0., 0., -s, c, 0., 0., 0., 0., 1., 0., x, y, 0., 1.],
        clips: Vec::new(),
        commands: vec![PaintCommand::FillRect {
            rect: CssRect {
                x: 0.,
                y: -width / 2.,
                width: length,
                height: width,
            },
            color,
        }],
    }
}
pub(crate) fn option_highlights(
    snapshot: &PaintSnapshot,
) -> std::collections::BTreeMap<u64, [u8; 4]> {
    let mut result = std::collections::BTreeMap::new();
    for node in &snapshot.nodes {
        let Some(PaintControl::Select {
            active_option,
            selected,
            open,
            multiple,
            display_size,
            disabled,
            ..
        }) = &node.control
        else {
            continue;
        };
        if node
            .style
            .get("appearance")
            .is_some_and(|value| matches!(value.as_str(), "none" | "base-select"))
        {
            continue;
        }
        let mut color = node
            .style
            .get("accent-color")
            .and_then(|value| crate::parse_color(value))
            .unwrap_or([0, 102, 204, 255]);
        color[3] = if *disabled { 35 } else { 80 };
        if *open {
            if let Some(option) = active_option {
                result.insert(*option, color);
            }
        } else if *multiple || *display_size > 1 {
            for option in selected {
                result.insert(*option, color);
            }
        }
    }
    result
}

pub(crate) fn commands(
    node: &PaintNodeSnapshot,
    model: &BoxModel,
) -> Result<Vec<PaintCommand>, RenderError> {
    let Some(control) = &node.control else {
        return Ok(Vec::new());
    };
    if node.tag == "::picker(select)" {
        return Ok(Vec::new());
    }
    if node
        .style
        .get("appearance")
        .is_some_and(|value| value == "none" || value == "base-select")
        || node
            .style
            .get("visibility")
            .is_some_and(|value| value != "visible")
    {
        return Ok(Vec::new());
    }
    let rect = model.content_box;
    if ![rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|v| v.is_finite())
        || rect.width <= 0.
        || rect.height <= 0.
    {
        return Ok(Vec::new());
    }
    let accent = node
        .style
        .get("accent-color")
        .and_then(|value| crate::parse_color(value))
        .unwrap_or([0, 102, 204, 255]);
    let disabled = matches!(
        control,
        PaintControl::Checkbox { disabled: true, .. }
            | PaintControl::Radio { disabled: true, .. }
            | PaintControl::Range { disabled: true, .. }
            | PaintControl::Color { disabled: true, .. }
            | PaintControl::Select { disabled: true, .. }
    );
    let accent = if disabled {
        [150, 150, 150, 255]
    } else {
        accent
    };
    let border = if disabled {
        [175, 175, 175, 255]
    } else {
        [105, 105, 105, 255]
    };
    let vertical = node
        .style
        .get("writing-mode")
        .is_some_and(|v| v != "horizontal-tb");
    let rtl = node.style.get("direction").is_some_and(|v| v == "rtl");
    let reversed = if node
        .style
        .get("writing-mode")
        .is_some_and(|v| v == "sideways-lr")
    {
        !rtl
    } else {
        rtl
    };
    let valid = |fraction: f64| {
        if fraction.is_finite() && (0.0..=1.0).contains(&fraction) {
            Ok(fraction)
        } else {
            Err(RenderError::Encode(
                "invalid native control fraction".into(),
            ))
        }
    };
    let part = |mut rect: CssRect, fraction: f64| {
        if vertical {
            let length = rect.height * fraction;
            if reversed {
                rect.y += rect.height - length;
            }
            rect.height = length;
        } else {
            let length = rect.width * fraction;
            if reversed {
                rect.x += rect.width - length;
            }
            rect.width = length;
        }
        rect
    };
    let mut out = Vec::new();
    match control {
        PaintControl::Meter { fraction, .. }
        | PaintControl::Progress {
            fraction: Some(fraction),
        } => {
            let fraction = valid(*fraction)?;
            let track = if vertical {
                CssRect {
                    x: rect.x + rect.width * 0.25,
                    width: rect.width * 0.5,
                    ..rect
                }
            } else {
                CssRect {
                    y: rect.y + rect.height * 0.25,
                    height: rect.height * 0.5,
                    ..rect
                }
            };
            let color = match control {
                PaintControl::Meter { region, .. } => match region {
                    PaintMeterRegion::Optimum => [44, 155, 67, 255],
                    PaintMeterRegion::Suboptimal => [218, 163, 24, 255],
                    PaintMeterRegion::EvenLessGood => [195, 48, 48, 255],
                },
                _ => accent,
            };
            let radius = track.width.min(track.height) / 2.;
            out.push(fill(track, border, radius));
            let inner = inset(track, 1.);
            out.push(fill(inner, [230, 230, 230, 255], (radius - 1.).max(0.)));
            out.push(fill(part(inner, fraction), color, (radius - 1.).max(0.)));
        }
        PaintControl::Progress { fraction: None } => {
            let track = if vertical {
                CssRect {
                    x: rect.x + rect.width * 0.25,
                    width: rect.width * 0.5,
                    ..rect
                }
            } else {
                CssRect {
                    y: rect.y + rect.height * 0.25,
                    height: rect.height * 0.5,
                    ..rect
                }
            };
            let radius = track.width.min(track.height) / 2.;
            out.push(fill(track, border, radius));
            out.push(fill(
                inset(track, 1.),
                [230, 230, 230, 255],
                (radius - 1.).max(0.),
            ));
            let mut middle = part(inset(track, 1.), 0.3);
            if vertical {
                middle.y += track.height * 0.35
            } else {
                middle.x += track.width * 0.35
            }
            out.push(fill(middle, accent, radius));
        }
        PaintControl::Checkbox {
            checked,
            indeterminate,
            ..
        } => {
            let rect = model.border_box;
            let radius = 2.;
            out.push(fill(rect, border, radius));
            out.push(fill(
                inset(rect, 1.),
                if *checked || *indeterminate {
                    accent
                } else {
                    [255, 255, 255, 255]
                },
                1.,
            ));
            if *indeterminate {
                out.push(PaintCommand::FillRect {
                    rect: CssRect {
                        x: rect.x + rect.width * 0.2,
                        y: rect.y + rect.height * 0.43,
                        width: rect.width * 0.6,
                        height: rect.height * 0.14,
                    },
                    color: [255; 4],
                });
            } else if *checked {
                let w = rect.width;
                let h = rect.height;
                out.push(segment(
                    rect.x + w * 0.2,
                    rect.y + h * 0.5,
                    w * 0.28,
                    w.min(h) * 0.13,
                    std::f64::consts::FRAC_PI_4,
                    [255; 4],
                ));
                out.push(segment(
                    rect.x + w * 0.39,
                    rect.y + h * 0.69,
                    w * 0.55,
                    w.min(h) * 0.13,
                    -std::f64::consts::FRAC_PI_4,
                    [255; 4],
                ));
            }
        }
        PaintControl::Radio { checked, .. } => {
            let rect = model.border_box;
            let radius = rect.width.min(rect.height) / 2.;
            out.push(fill(rect, border, radius));
            out.push(fill(inset(rect, 1.), [255; 4], radius));
            if *checked {
                out.push(fill(
                    inset(rect, rect.width.min(rect.height) * 0.25),
                    accent,
                    radius,
                ));
            }
        }
        PaintControl::Color { rgba, .. } => {
            out.push(fill(rect, border, 2.));
            out.push(fill(
                inset(rect, 2.),
                if disabled {
                    [
                        rgba[0] / 2 + 96,
                        rgba[1] / 2 + 96,
                        rgba[2] / 2 + 96,
                        rgba[3],
                    ]
                } else {
                    *rgba
                },
                1.,
            ));
        }
        PaintControl::Range {
            fraction, ticks, ..
        } => {
            let fraction = valid(*fraction)?;
            if ticks.len() > 1024 {
                return Err(RenderError::Encode(
                    "native range tick budget exceeded".into(),
                ));
            }
            for tick in ticks {
                let tick = valid(*tick)?;
                let tick = if reversed { 1. - tick } else { tick };
                let diameter = rect.width.min(rect.height).min(16.);
                let mark = if vertical {
                    CssRect {
                        x: rect.x,
                        y: rect.y + diameter / 2. + (rect.height - diameter) * tick - 1.,
                        width: 4.,
                        height: 2.,
                    }
                } else {
                    CssRect {
                        x: rect.x + diameter / 2. + (rect.width - diameter) * tick - 1.,
                        y: rect.bottom() - 4.,
                        width: 2.,
                        height: 4.,
                    }
                };
                out.push(fill(mark, border, 0.));
            }
            let track = if vertical {
                CssRect {
                    x: rect.x + rect.width / 2. - 2.,
                    width: 4.,
                    ..rect
                }
            } else {
                CssRect {
                    y: rect.y + rect.height / 2. - 2.,
                    height: 4.,
                    ..rect
                }
            };
            out.push(fill(track, [200, 200, 200, 255], 2.));
            out.push(fill(part(track, fraction), accent, 2.));
            let f = if reversed { 1. - fraction } else { fraction };
            let diameter = rect.width.min(rect.height).min(16.);
            let thumb = if vertical {
                CssRect {
                    x: rect.x + (rect.width - diameter) / 2.,
                    y: rect.y + (rect.height - diameter) * f,
                    width: diameter,
                    height: diameter,
                }
            } else {
                CssRect {
                    x: rect.x + (rect.width - diameter) * f,
                    y: rect.y + (rect.height - diameter) / 2.,
                    width: diameter,
                    height: diameter,
                }
            };
            out.push(fill(thumb, accent, diameter / 2.));
        }
        PaintControl::Select {
            multiple: false,
            open,
            ..
        } => {
            let box_ = model.padding_box;
            let x = if rtl { box_.x + 8. } else { box_.right() - 10. };
            let y = box_.y + box_.height / 2.;
            let sign = if *open { -1. } else { 1. };
            out.push(segment(
                x - 3.,
                y - sign * 1.5,
                4.2,
                1.5,
                sign * std::f64::consts::FRAC_PI_4,
                border,
            ));
            out.push(segment(
                x,
                y + sign * 1.5,
                4.2,
                1.5,
                -sign * std::f64::consts::FRAC_PI_4,
                border,
            ));
        }
        PaintControl::Select { multiple: true, .. } => {}
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_meter_appearance_none_differs_from_native_auto() {
        let mut node = crate::tests::paint_node(
            1,
            None,
            "meter",
            CssRect {
                x: 0.,
                y: 0.,
                width: 80.,
                height: 16.,
            },
        );
        node.control = Some(PaintControl::Meter {
            fraction: 0.5,
            region: PaintMeterRegion::Optimum,
        });
        let model = node.geometry.box_model.as_ref().unwrap();
        assert!(!commands(&node, model).unwrap().is_empty());
        let model = model.clone();
        node.style.insert("appearance".into(), "none".into());
        assert!(commands(&node, &model).unwrap().is_empty());
    }
    #[test]
    fn invalid_native_fraction_is_an_error_not_a_successful_empty_theme() {
        let mut node = crate::tests::paint_node(
            1,
            None,
            "meter",
            CssRect {
                x: 0.,
                y: 0.,
                width: 80.,
                height: 16.,
            },
        );
        node.control = Some(PaintControl::Meter {
            fraction: f64::NAN,
            region: PaintMeterRegion::Optimum,
        });
        assert!(commands(&node, node.geometry.box_model.as_ref().unwrap()).is_err());
    }
    #[test]
    fn native_picker_highlight_uses_active_option_without_changing_selectedness() {
        let mut snapshot = crate::tests::empty_snapshot();
        let mut select = crate::tests::paint_node(
            91,
            None,
            "select",
            CssRect {
                x: 0.,
                y: 0.,
                width: 100.,
                height: 30.,
            },
        );
        select.control = Some(PaintControl::Select {
            active_option: Some(93),
            open: true,
            multiple: false,
            disabled: false,
            selected: vec![92],
            label: "One".into(),
            display_size: 1,
        });
        snapshot.nodes.push(select);
        let highlights = option_highlights(&snapshot);
        assert!(highlights.contains_key(&93));
        assert!(!highlights.contains_key(&92));
    }
    #[test]
    fn native_checkbox_checked_and_indeterminate_have_distinct_commands() {
        let mut node = crate::tests::paint_node(
            1,
            None,
            "input",
            CssRect {
                x: 0.,
                y: 0.,
                width: 16.,
                height: 16.,
            },
        );
        node.control = Some(PaintControl::Checkbox {
            checked: true,
            indeterminate: false,
            disabled: false,
        });
        let checked = commands(&node, node.geometry.box_model.as_ref().unwrap()).unwrap();
        node.control = Some(PaintControl::Checkbox {
            checked: false,
            indeterminate: true,
            disabled: false,
        });
        let mixed = commands(&node, node.geometry.box_model.as_ref().unwrap()).unwrap();
        assert_ne!(checked, mixed);
    }
}

#[cfg(test)]
mod color_range_tests {
    use super::*;
    #[test]
    fn native_color_well_and_range_ticks_generate_typed_paint_commands() {
        let mut node = crate::tests::paint_node(
            1,
            None,
            "input",
            CssRect {
                x: 0.,
                y: 0.,
                width: 120.,
                height: 24.,
            },
        );
        let model = node.geometry.box_model.clone().unwrap();
        node.control = Some(PaintControl::Color {
            rgba: [18, 52, 86, 255],
            disabled: false,
        });
        assert!(
            commands(&node, &model)
                .unwrap()
                .iter()
                .any(|command| matches!(
                    command,
                    PaintCommand::FillRoundedRect {
                        color: [18, 52, 86, 255],
                        ..
                    }
                ))
        );
        node.control = Some(PaintControl::Range {
            fraction: 0.5,
            disabled: false,
            ticks: vec![0., 0.5, 1.],
        });
        assert_eq!(commands(&node, &model).unwrap().len(), 6);
        node.style.insert("appearance".into(), "none".into());
        assert!(commands(&node, &model).unwrap().is_empty());
    }
}
