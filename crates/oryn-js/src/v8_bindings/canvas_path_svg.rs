//! SVG path-data parsing with valid-prefix recovery for Path2D.
use super::canvas_path::CanvasPath;
use kurbo::{Arc, Point, SvgArc};

struct Input<'a> {
    text: &'a str,
    position: usize,
}
impl Input<'_> {
    fn whitespace(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.position)
            .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.position += 1;
        }
    }
    fn separator(&mut self, comma: bool) {
        self.whitespace();
        if comma && self.text.as_bytes().get(self.position) == Some(&b',') {
            self.position += 1;
            self.whitespace();
        }
    }
    fn number(&mut self, comma: bool, flag: bool) -> Option<f64> {
        self.separator(comma);
        let bytes = self.text.as_bytes();
        let start = self.position;
        if flag {
            let value = match bytes.get(self.position)? {
                b'0' => 0.0,
                b'1' => 1.0,
                _ => return None,
            };
            self.position += 1;
            return Some(value);
        }
        if bytes
            .get(self.position)
            .is_some_and(|c| matches!(c, b'+' | b'-'))
        {
            self.position += 1;
        }
        let mut digits = 0;
        while bytes.get(self.position).is_some_and(u8::is_ascii_digit) {
            digits += 1;
            self.position += 1;
        }
        if bytes.get(self.position) == Some(&b'.') {
            self.position += 1;
            while bytes.get(self.position).is_some_and(u8::is_ascii_digit) {
                digits += 1;
                self.position += 1;
            }
        }
        if digits == 0 {
            return None;
        }
        if bytes
            .get(self.position)
            .is_some_and(|c| matches!(c, b'e' | b'E'))
        {
            self.position += 1;
            if bytes
                .get(self.position)
                .is_some_and(|c| matches!(c, b'+' | b'-'))
            {
                self.position += 1;
            }
            let exponent = self.position;
            while bytes.get(self.position).is_some_and(u8::is_ascii_digit) {
                self.position += 1;
            }
            if exponent == self.position {
                return None;
            }
        }
        let value = self.text[start..self.position].parse::<f64>().ok()?;
        value.is_finite().then_some(value)
    }
}

pub(super) fn parse(text: &str) -> CanvasPath {
    let mut input = Input { text, position: 0 };
    let mut path = CanvasPath::default();
    let mut current = Point::ORIGIN;
    let mut start = Point::ORIGIN;
    let mut command = None;
    let mut previous = b' ';
    let mut control = Point::ORIGIN;
    let mut initialized = false;
    loop {
        input.whitespace();
        let Some(&next) = text.as_bytes().get(input.position) else {
            break;
        };
        let explicit = next.is_ascii_alphabetic();
        if explicit {
            command = Some(next);
            input.position += 1;
        }
        let Some(cmd) = command else {
            break;
        };
        let upper = cmd.to_ascii_uppercase();
        if !initialized && upper != b'M' {
            break;
        }
        if upper == b'Z' {
            path.close();
            current = start;
            previous = b'Z';
            command = None;
            continue;
        }
        let count = match upper {
            b'M' | b'L' | b'T' => 2,
            b'H' | b'V' => 1,
            b'Q' | b'S' => 4,
            b'C' => 6,
            b'A' => 7,
            _ => break,
        };
        let mut numbers = [0.0; 7];
        let mut valid = true;
        for (index, number) in numbers.iter_mut().enumerate().take(count) {
            let Some(value) = input.number(
                index != 0 || !explicit,
                upper == b'A' && (index == 3 || index == 4),
            ) else {
                valid = false;
                break;
            };
            *number = value;
        }
        if !valid {
            break;
        }
        let n = numbers;
        let base = if cmd.is_ascii_lowercase() {
            current
        } else {
            Point::ORIGIN
        };
        let point = |x, y| Point::new(base.x + x, base.y + y);
        // Commit only complete argument groups; malformed later groups keep
        // the preceding segments, including implicit repetitions.
        match upper {
            b'M' => {
                current = point(n[0], n[1]);
                if !current.is_finite() {
                    break;
                }
                path.move_to(current);
                start = current;
                initialized = true;
                command = Some(if cmd == b'm' { b'l' } else { b'L' });
            }
            b'L' => {
                current = point(n[0], n[1]);
                path.line_to(current);
            }
            b'H' => {
                current = Point::new(base.x + n[0], current.y);
                path.line_to(current);
            }
            b'V' => {
                current = Point::new(current.x, base.y + n[0]);
                path.line_to(current);
            }
            b'Q' => {
                let cp = point(n[0], n[1]);
                let end = point(n[2], n[3]);
                path.quad_to(cp, end);
                control = cp;
                current = end;
            }
            b'T' => {
                let cp = if matches!(previous, b'Q' | b'T') {
                    Point::new(2.0 * current.x - control.x, 2.0 * current.y - control.y)
                } else {
                    current
                };
                let end = point(n[0], n[1]);
                path.quad_to(cp, end);
                control = cp;
                current = end;
            }
            b'C' => {
                let a = point(n[0], n[1]);
                let b = point(n[2], n[3]);
                let end = point(n[4], n[5]);
                path.curve_to(a, b, end);
                control = b;
                current = end;
            }
            b'S' => {
                let a = if matches!(previous, b'C' | b'S') {
                    Point::new(2.0 * current.x - control.x, 2.0 * current.y - control.y)
                } else {
                    current
                };
                let b = point(n[0], n[1]);
                let end = point(n[2], n[3]);
                path.curve_to(a, b, end);
                control = b;
                current = end;
            }
            b'A' => {
                if n[0] < 0.0 || n[1] < 0.0 {
                    break;
                }
                let end = point(n[5], n[6]);
                if end != current {
                    let arc = SvgArc {
                        from: current,
                        to: end,
                        radii: kurbo::Vec2::new(n[0], n[1]),
                        x_rotation: n[2].to_radians(),
                        large_arc: n[3] != 0.0,
                        sweep: n[4] != 0.0,
                    };
                    if let Some(arc) = Arc::from_svg_arc(&arc) {
                        if !arc.center.is_finite()
                            || !arc.radii.is_finite()
                            || !arc.start_angle.is_finite()
                            || !arc.sweep_angle.is_finite()
                        {
                            break;
                        }
                        path.ellipse(
                            (arc.center.x, arc.center.y),
                            (arc.radii.x, arc.radii.y),
                            arc.x_rotation,
                            arc.start_angle,
                            arc.start_angle + arc.sweep_angle,
                            arc.sweep_angle < 0.0,
                            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                        );
                    } else {
                        path.line_to(end);
                    }
                }
                current = end;
            }
            _ => unreachable!(),
        }
        if !current.is_finite() {
            break;
        }
        previous = upper;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::super::canvas_path::contains;
    use super::*;
    #[test]
    fn retains_complete_segments_before_errors() {
        for text in [
            "M0 0L4 0 4 4 0 4z L1",
            "M0 0L4 0 4 4 0 4z INVALID",
            "M0 0L4 0 4 4 0 4z ,M8 8",
            "M0 0L4 0 4 4 0 4z A1 1 0 2 0 9 9",
        ] {
            assert!(contains(&parse(text).closed(), 2.0, 2.0, false), "{text}");
        }
        for text in ["L0 0 4 0 4 4z", "M,0 0L4 0 4 4z", "M0 0L4,", "M0 0L1e 4"] {
            assert!(!contains(&parse(text).closed(), 2.0, 1.0, false), "{text}");
        }
    }
    #[test]
    fn smooth_commands_reflect_only_the_matching_curve_family() {
        let actual = parse("M0 0Q2 4 4 0S6 4 8 0").closed();
        let expected = parse("M0 0Q2 4 4 0C4 0 6 4 8 0").closed();
        assert_eq!(actual.elements(), expected.elements());
        let actual = parse("M0 0C1 4 3 4 4 0T8 0").closed();
        let expected = parse("M0 0C1 4 3 4 4 0Q4 0 8 0").closed();
        assert_eq!(actual.elements(), expected.elements());
        assert_eq!(
            parse("M0 0Q2 4 4 0T8 0").closed().elements(),
            parse("M0 0Q2 4 4 0Q6 -4 8 0").closed().elements()
        );
    }

    #[test]
    fn compact_numbers_relative_repetition_and_close() {
        let path = parse("m1 1 4 0v4h-4z m6 0h2v2h-2z").closed();
        assert!(contains(&path, 2.0, 2.0, false));
        assert!(contains(&path, 8.0, 2.0, false));
        assert!(!contains(&path, 6.0, 2.0, false));
        assert!(contains(
            &parse("M.5.5L4.5.5 4.5 4.5.5 4.5z").closed(),
            2.0,
            2.0,
            false
        ));
    }
}
