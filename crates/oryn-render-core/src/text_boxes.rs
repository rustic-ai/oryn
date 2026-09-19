//! First-letter boxes use the same native background/border/shadow primitives
//! as ordinary elements, without inventing DOM or accessibility identities.
use crate::{PaintCommand, RenderError, backgrounds, borders, shadows};
use oryn_common::v4::{PaintBorderEdge, PaintGlyphRun, PaintSnapshot, PaintTextFragment};

pub(super) fn commands(
    snapshot: &PaintSnapshot,
    fragment: &PaintTextFragment,
    run: &PaintGlyphRun,
) -> Result<Vec<PaintCommand>, RenderError> {
    let mut output = Vec::new();
    for source in [run.line_box.as_ref(), run.text_box.as_ref()]
        .into_iter()
        .flatten()
    {
        output.extend(box_commands(snapshot, fragment, source)?);
    }
    Ok(output)
}
fn box_commands(
    snapshot: &PaintSnapshot,
    fragment: &PaintTextFragment,
    source: &oryn_common::v4::PaintTextBox,
) -> Result<Vec<PaintCommand>, RenderError> {
    let mut source = source.clone();
    source.translate(fragment.rect.x, fragment.rect.y);
    let model = &source.model;
    let style = &source.style;
    for rect in [model.border_box, model.padding_box, model.content_box] {
        if ![rect.x, rect.y, rect.width, rect.height]
            .iter()
            .all(|value| value.is_finite())
            || rect.width < 0.
            || rect.height < 0.
        {
            return Err(RenderError::Font("invalid native text pseudo box".into()));
        }
    }
    if [
        model.border.top,
        model.border.right,
        model.border.bottom,
        model.border.left,
        model.padding.top,
        model.padding.right,
        model.padding.bottom,
        model.padding.left,
    ]
    .iter()
    .any(|value| !value.is_finite() || *value < 0.)
    {
        return Err(RenderError::Font(
            "invalid native text pseudo box edges".into(),
        ));
    }
    let radii = borders::radii(style, model.border_box);
    let font = style
        .get("font-size")
        .and_then(|value| value.strip_suffix("px"))
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(16.);
    let shadows = style
        .get("box-shadow")
        .filter(|value| value.as_str() != "none")
        .and_then(|value| {
            shadows::parse(
                value,
                style.get("color").map(String::as_str).unwrap_or("black"),
                font,
            )
        })
        .unwrap_or_default();
    let mut output = Vec::new();
    for shadow in shadows
        .iter()
        .rev()
        .filter(|shadow| !shadow.inset && shadow.color[3] != 0)
    {
        output.push(PaintCommand::BoxShadow {
            rect: model.border_box,
            shadow: shadow.clone(),
            radii,
        });
    }
    if let Some(color) = style
        .get("background-color")
        .and_then(|value| oryn_common::color::parse_color(value))
        .filter(|color| color[3] != 0)
    {
        let (rect, radii) = backgrounds::color_clip(style, model, radii);
        let command = if radii.iter().any(|radius| *radius > 0.) {
            PaintCommand::FillRoundedRect { rect, color, radii }
        } else {
            PaintCommand::FillRect { rect, color }
        };
        if backgrounds::color_uses_text(style) {
            output.push(PaintCommand::TextClip {
                owner: source.owner,
                fragment: Some(model.border_box),
                commands: vec![command],
            });
        } else {
            output.push(command);
        }
    }
    match backgrounds::commands(snapshot, style, model, radii, None, false, None) {
        Ok(mut commands) => {
            for command in &mut commands {
                if let PaintCommand::TextClip {
                    owner, fragment, ..
                } = command
                {
                    *owner = source.owner;
                    *fragment = Some(model.border_box);
                }
            }
            output.extend(commands);
        }
        Err(error) if error.contains("resource not captured") => {} // Failed/pending images retain the CSS background color.
        Err(error) => {
            return Err(RenderError::Font(format!(
                "text pseudo background: {error}"
            )));
        }
    }
    for shadow in shadows
        .iter()
        .rev()
        .filter(|shadow| shadow.inset && shadow.color[3] != 0)
    {
        let mut inner = radii;
        let widths = [
            model.border.left,
            model.border.top,
            model.border.right,
            model.border.top,
            model.border.right,
            model.border.bottom,
            model.border.left,
            model.border.bottom,
        ];
        for (radius, width) in inner.iter_mut().zip(widths) {
            *radius = (*radius - width).max(0.);
        }
        output.push(PaintCommand::BoxShadow {
            rect: model.padding_box,
            shadow: shadow.clone(),
            radii: inner,
        });
    }
    let widths = [
        model.border.top,
        model.border.right,
        model.border.bottom,
        model.border.left,
    ];
    let edges: [PaintBorderEdge; 4] = std::array::from_fn(|index| {
        let side = ["top", "right", "bottom", "left"][index];
        PaintBorderEdge {
            width: widths[index],
            style: style
                .get(&format!("border-{side}-style"))
                .cloned()
                .unwrap_or_else(|| "none".into()),
            color: style
                .get(&format!("border-{side}-color"))
                .filter(|color| color.as_str() != "currentcolor")
                .or_else(|| style.get("color"))
                .cloned()
                .unwrap_or_else(|| "black".into()),
        }
    });
    if edges
        .iter()
        .any(|edge| edge.width > 0. && !matches!(edge.style.as_str(), "none" | "hidden"))
    {
        output.push(PaintCommand::BorderEdges {
            rect: model.border_box,
            edges,
            radii,
        });
    }
    Ok(output)
}
