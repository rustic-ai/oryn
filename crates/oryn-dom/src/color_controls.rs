//! HTML color-state values use the native CSS4 parser and color-space conversion.
//! Serialization is distinct from the sRGB raster projection of a color well.
use super::*;
use color::ColorSpaceTag;
fn byte(value: f32) -> u8 {
    (value.clamp(0., 1.) * 255.).round() as u8
}
pub fn input_color_components(value: &str, p3: bool, alpha: bool) -> [f32; 4] {
    let mut parsed =
        color::parse_color(value).unwrap_or_else(|_| color::parse_color("black").unwrap());
    for index in 0..4 {
        if parsed.flags.missing().contains(index) {
            parsed.components[index] = 0.;
        }
    }
    parsed.flags = Default::default();
    // Legacy sRGB functional/hex/named inputs resolve through their 8-bit CSS
    // representation before conversion; color() and wide-gamut inputs retain floats.
    if parsed.cs == ColorSpaceTag::Srgb
        && !value
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("color(")
    {
        for channel in &mut parsed.components {
            *channel = f32::from(byte(*channel)) / 255.;
        }
    }
    let mut channels = parsed
        .convert(if p3 {
            ColorSpaceTag::DisplayP3
        } else {
            ColorSpaceTag::Srgb
        })
        .components;
    for channel in &mut channels {
        if !channel.is_finite() || channel.abs() < 0.000001 {
            *channel = 0.;
        } else if (*channel - 1.).abs() < 0.000001 {
            *channel = 1.;
        }
    }
    if !p3 {
        for channel in &mut channels {
            *channel = f32::from(byte(*channel)) / 255.;
        }
    }
    channels[3] = if alpha { channels[3].clamp(0., 1.) } else { 1. };
    channels
}
pub fn serialize_input_color(channels: [f32; 4], p3: bool, alpha: bool) -> String {
    if !p3 && !alpha {
        return format!(
            "#{:02x}{:02x}{:02x}",
            byte(channels[0]),
            byte(channels[1]),
            byte(channels[2])
        );
    }
    let channels = channels.map(|value| if value == 0. { 0. } else { value });
    let space = if p3 { "display-p3" } else { "srgb" };
    let suffix = if alpha && channels[3] != 1. {
        format!(" / {}", channels[3])
    } else {
        String::new()
    };
    format!(
        "color({space} {} {} {}{suffix})",
        channels[0], channels[1], channels[2]
    )
}
pub fn input_color_rgba8(value: &str) -> [u8; 4] {
    let mut parsed =
        color::parse_color(value).unwrap_or_else(|_| color::parse_color("black").unwrap());
    for index in 0..4 {
        if parsed.flags.missing().contains(index) {
            parsed.components[index] = 0.;
        }
    }
    parsed.flags = Default::default();
    parsed.convert(ColorSpaceTag::Srgb).components.map(byte)
}
impl DomRuntime {
    pub fn input_color_space(&self, node: NodeId) -> Result<&'static str, DomError> {
        Ok(
            if self
                .attribute_in_null_namespace(node, "colorspace")?
                .is_some_and(|value| value.eq_ignore_ascii_case("display-p3"))
            {
                "display-p3"
            } else {
                "limited-srgb"
            },
        )
    }
    pub fn color_input_configuration(&self, node: NodeId) -> Result<(bool, bool), DomError> {
        Ok((
            self.input_color_space(node)? == "display-p3",
            self.attribute_in_null_namespace(node, "alpha")?.is_some(),
        ))
    }
    pub(super) fn sanitize_color_value(
        &self,
        node: NodeId,
        value: &str,
    ) -> Result<String, DomError> {
        let (p3, alpha) = self.color_input_configuration(node)?;
        Ok(serialize_input_color(
            input_color_components(value, p3, alpha),
            p3,
            alpha,
        ))
    }
    pub fn color_picker_value(
        &self,
        node: NodeId,
        draft: &str,
        field: u8,
        channel: u8,
    ) -> Result<String, DomError> {
        let (p3, alpha) = self.color_input_configuration(node)?;
        let mut channels = input_color_components(draft, p3, alpha);
        channels[field.min(if alpha { 3 } else { 2 }) as usize] = f32::from(channel) / 255.;
        Ok(serialize_input_color(channels, p3, alpha))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn css4_input_colors_preserve_alpha_and_display_p3_and_resolve_missing() {
        assert_eq!(
            serialize_input_color(
                input_color_components("crimson", false, false),
                false,
                false
            ),
            "#dc143c"
        );
        assert_eq!(
            serialize_input_color(
                input_color_components("color(display-p3 3 none .2 / .6)", true, true),
                true,
                true
            ),
            "color(display-p3 3 0 0.2 / 0.6)"
        );
        assert_eq!(
            serialize_input_color(
                input_color_components("transparent", false, true),
                false,
                true
            ),
            "color(srgb 0 0 0 / 0)"
        );
        let color = input_color_components("rgb(1,1,1,0.5)", false, true);
        assert!((color[3] - 128. / 255.).abs() < 0.00001);
    }
    #[test]
    fn nondirty_color_attribute_recovers_alpha_but_dirty_color_does_not() {
        let mut dom = html::parse("<input id=c type=color value='#ffffff08'>")
            .into_runtime()
            .unwrap();
        let c = dom.find_by_id("c").unwrap();
        assert_eq!(dom.control_value(c).unwrap(), "#ffffff");
        dom.set_attribute(c, "alpha", "").unwrap();
        assert!(
            (input_color_components(&dom.control_value(c).unwrap(), false, true)[3] - 8. / 255.)
                .abs()
                < 0.00001
        );
        dom.set_value(c, "rgba(255,255,255,.5)", false).unwrap();
        dom.remove_attribute(c, "alpha").unwrap();
        dom.set_attribute(c, "alpha", "").unwrap();
        assert_eq!(dom.control_value(c).unwrap(), "color(srgb 1 1 1)");
    }
}
