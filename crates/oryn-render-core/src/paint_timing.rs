//! Paint Timing's content milestone uses native content commands and the same
//! clipping, opacity, mask and filter rasterization as the displayed scene.
use super::*;
fn image_source(source: &BackgroundSource) -> bool {
    match source {
        BackgroundSource::Image { .. } => true,
        BackgroundSource::CrossFade(parts) => parts
            .iter()
            .any(|(weight, source)| *weight > 0. && image_source(source)),
        _ => false,
    }
}
fn content(commands: &[PaintCommand]) -> Vec<PaintCommand> {
    commands
        .iter()
        .filter_map(|command| {
            let mut result = command.clone();
            match &mut result {
                PaintCommand::Text { fragment, color }
                    if !fragment.text.trim().is_empty() && color[3] > 0 => {}
                PaintCommand::Image { .. }
                | PaintCommand::SvgImage { .. }
                | PaintCommand::BuiltinImage { .. } => {}
                PaintCommand::Background { source, .. } if image_source(source) => {}
                PaintCommand::Canvas { color, layers } => {
                    *color = [0; 4];
                    layers.retain(|(_, source)| image_source(source));
                    if layers.is_empty() {
                        return None;
                    }
                }
                PaintCommand::Opacity { opacity, commands } => {
                    if *opacity <= 0. {
                        return None;
                    }
                    *commands = content(commands);
                    if commands.is_empty() {
                        return None;
                    }
                }
                PaintCommand::Projected { commands, .. }
                | PaintCommand::Mask { commands, .. }
                | PaintCommand::Filter { commands, .. }
                | PaintCommand::TextClip { commands, .. } => {
                    *commands = content(commands);
                    if commands.is_empty() {
                        return None;
                    }
                }
                // Solid backgrounds, borders and other documents are not contentful
                // content in this document's Performance Timeline.
                _ => return None,
            }
            Some(result)
        })
        .collect()
}
pub fn scene_has_contentful_paint(
    snapshot: &PaintSnapshot,
    scene: &DisplayList,
    mut options: RenderOptions,
) -> Result<bool, RenderError> {
    let dimensions = super::dimensions(snapshot, options)?;
    if dimensions != (scene.width, scene.height) {
        return Err(RenderError::Encode(
            "paint timing scene dimensions mismatch".into(),
        ));
    }
    let commands = content(&scene.commands);
    if commands.is_empty() {
        return Ok(false);
    }
    let content_scene = DisplayList {
        contract_version: scene.contract_version,
        width: scene.width,
        height: scene.height,
        commands,
        diagnostics: Vec::new(),
    };
    options.background = [0; 4];
    let pixels = super::rasterize(snapshot, &content_scene, options)?;
    Ok(pixels.chunks_exact(4).any(|pixel| pixel[3] != 0))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transparent_groups_and_noncontent_paint_do_not_contribute() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
        };
        let commands = vec![
            PaintCommand::FillRect {
                rect,
                color: [255; 4],
            },
            PaintCommand::Opacity {
                opacity: 0.,
                commands: vec![PaintCommand::Image {
                    rect,
                    resource: 0,
                    rendering: ImageRendering::default(),
                    clip: None,
                }],
            },
            PaintCommand::EmbeddedDocument { rect, frame: 0 },
        ];
        assert!(content(&commands).is_empty());
    }
    #[test]
    fn content_keeps_clipping_and_effect_wrappers() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
        };
        let image = PaintCommand::Image {
            rect,
            resource: 0,
            rendering: ImageRendering::default(),
            clip: Some(rect),
        };
        let commands = vec![PaintCommand::Opacity {
            opacity: 0.5,
            commands: vec![image],
        }];
        assert_eq!(content(&commands), commands);
    }
}

fn frame_coverage(commands: &[PaintCommand], wanted: usize) -> Vec<PaintCommand> {
    commands
        .iter()
        .filter_map(|command| {
            let mut result = command.clone();
            match &mut result {
                PaintCommand::EmbeddedDocument { rect, frame } if *frame == wanted => {
                    result = PaintCommand::FillRect {
                        rect: *rect,
                        color: [255; 4],
                    }
                }
                PaintCommand::Opacity { opacity, commands } => {
                    if *opacity <= 0. {
                        return None;
                    }
                    *commands = frame_coverage(commands, wanted);
                    if commands.is_empty() {
                        return None;
                    }
                }
                PaintCommand::Projected { commands, .. }
                | PaintCommand::Mask { commands, .. }
                | PaintCommand::Filter { commands, .. }
                | PaintCommand::TextClip { commands, .. } => {
                    *commands = frame_coverage(commands, wanted);
                    if commands.is_empty() {
                        return None;
                    }
                }
                _ => return None,
            }
            Some(result)
        })
        .collect()
}
/// Whether a child document has any presented coverage after the embedding
/// document's native clipping, opacity, masking and projection effects.
pub fn scene_frame_is_presented(
    snapshot: &PaintSnapshot,
    scene: &DisplayList,
    frame: usize,
    mut options: RenderOptions,
) -> Result<bool, RenderError> {
    if frame >= snapshot.frames.len() {
        return Ok(false);
    }
    let commands = frame_coverage(&scene.commands, frame);
    if commands.is_empty() {
        return Ok(false);
    }
    let marker = DisplayList {
        contract_version: scene.contract_version,
        width: scene.width,
        height: scene.height,
        commands,
        diagnostics: Vec::new(),
    };
    options.background = [0; 4];
    let pixels = super::rasterize(snapshot, &marker, options)?;
    Ok(pixels.chunks_exact(4).any(|pixel| pixel[3] != 0))
}
#[cfg(test)]
mod frame_coverage_tests {
    use super::*;
    #[test]
    fn embedding_effects_and_requested_frame_identity_control_coverage() {
        let rect = CssRect {
            x: 0.,
            y: 0.,
            width: 10.,
            height: 10.,
        };
        let frame = PaintCommand::EmbeddedDocument { rect, frame: 2 };
        assert!(frame_coverage(&[frame.clone()], 1).is_empty());
        assert!(
            frame_coverage(
                &[PaintCommand::Opacity {
                    opacity: 0.,
                    commands: vec![frame.clone()]
                }],
                2
            )
            .is_empty()
        );
        assert!(matches!(
            &frame_coverage(&[frame], 2)[0],
            PaintCommand::FillRect { .. }
        ));
    }
}
