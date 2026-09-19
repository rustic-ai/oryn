// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#[cfg(feature = "libm")]
#[allow(unused_imports)]
use core_maths::CoreFloat;

/// Distance between preserved tab stops, as a space count or a layout length.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum TabSize {
    /// A multiple of the current font's space advance, including spacing.
    Spaces(f32),
    /// An absolute distance in layout units.
    Length(f32),
}
impl Default for TabSize {
    fn default() -> Self { Self::Spaces(8.) }
}
impl TabSize {
    pub(crate) fn scale(self, scale: f32) -> Self {
        match self { Self::Spaces(_) => self, Self::Length(value) => Self::Length(value * scale) }
    }

    pub(crate) fn advance(self, position: f32, space: f32) -> f32 {
        let space = space.max(0.);
        let step = match self { Self::Spaces(value) => value * space, Self::Length(value) => value };
        if !step.is_finite() || step <= 0. || !position.is_finite() { return 0.; }
        let mut gap = ((position / step).floor() + 1.) * step - position;
        let minimum = space * 0.5;
        if gap < minimum { gap += ((minimum - gap) / step).ceil() * step; }
        gap.max(0.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tabs_use_next_stop_with_minimum_space_and_zero_size() {
        assert_eq!(TabSize::Spaces(4.).advance(10., 5.), 10.);
        assert_eq!(TabSize::Spaces(4.).advance(20., 5.), 20.);
        assert_eq!(TabSize::Spaces(4.).advance(19., 5.), 21.);
        assert_eq!(TabSize::Length(10.).advance(-2., 4.), 2.);
        assert_eq!(TabSize::Length(1.).advance(0.75, 8.), 4.25);
        assert_eq!(TabSize::Spaces(0.).advance(19., 5.), 0.);
        assert_eq!(TabSize::Length(0.).advance(19., 5.), 0.);
        assert_eq!(TabSize::Spaces(4.).scale(2.), TabSize::Spaces(4.));
        assert_eq!(TabSize::Length(10.).scale(2.), TabSize::Length(20.));
    }
}
