//! Native number/calendar parsing shared by validation and value sanitization.
use super::{DomError, DomRuntime, NodeId};
use num_bigint::BigInt;

const HTML: &str = "http://www.w3.org/1999/xhtml";
const DAY: i64 = 86_400_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NumericValidity {
    pub range_underflow: bool,
    pub range_overflow: bool,
    pub step_mismatch: bool,
}

pub(super) fn numeric_kind(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "number" | "range" | "date" | "month" | "week" | "time" | "datetime-local"
    )
}

// Attribute parsing follows the HTML prefix parser, while value sanitization
// requires the entire strict floating-point grammar (no leading + or spaces).
fn number(value: &str, strict: bool) -> Option<f64> {
    let input = if strict {
        value
    } else {
        value.trim_start_matches(['\t', '\n', '\x0c', '\r', ' '])
    };
    let b = input.as_bytes();
    let mut i = usize::from(b.first() == Some(&b'-') || (!strict && b.first() == Some(&b'+')));
    let start = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let integer_digits = i - start;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let fraction = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == fraction {
            if strict || integer_digits == 0 {
                return None;
            }
            return input[..i - 1].parse::<f64>().ok().filter(|n| n.is_finite());
        }
    } else if integer_digits == 0 {
        return None;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let exponent_start = i;
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let digits = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == digits {
            if strict {
                return None;
            }
            i = exponent_start;
        }
    }
    if strict && i != b.len() {
        return None;
    }
    input
        .get(..i)?
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}

fn digits(value: &str, width: usize) -> Option<i64> {
    (value.len() == width && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn year(value: &str) -> Option<i64> {
    (value.len() >= 4 && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse::<i64>().ok())
        .flatten()
        .filter(|y| *y > 0)
}

fn leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

fn month(value: &str) -> Option<(i64, i64)> {
    let (y, m) = value.split_once('-')?;
    let m = digits(m, 2)?;
    (1..=12).contains(&m).then_some((year(y)?, m))
}

// Proleptic Gregorian days before this year, relative to 1970-01-01.
fn year_days(y: i64) -> Option<i64> {
    let previous = y.checked_sub(1)?;
    previous
        .checked_mul(365)?
        .checked_add(previous / 4)?
        .checked_sub(previous / 100)?
        .checked_add(previous / 400)?
        .checked_sub(719_162)
}

fn date(value: &str) -> Option<i64> {
    let (ym, d) = value.rsplit_once('-')?;
    let (y, m) = month(ym)?;
    let d = digits(d, 2)?;
    let lengths = [
        31,
        if leap(y) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=lengths[(m - 1) as usize]).contains(&d) {
        return None;
    }
    let before: i64 = lengths[..(m - 1) as usize].iter().sum();
    year_days(y)?
        .checked_add(before)?
        .checked_add(d - 1)?
        .checked_mul(DAY)
}

fn time(value: &str) -> Option<i64> {
    let mut parts = value.split(':');
    let h = digits(parts.next()?, 2)?;
    let m = digits(parts.next()?, 2)?;
    let mut second = 0;
    let mut millis = 0;
    if let Some(s) = parts.next() {
        let (s, fraction) = s.split_once('.').map_or((s, None), |(s, f)| (s, Some(f)));
        second = digits(s, 2)?;
        if let Some(f) = fraction {
            if !(1..=3).contains(&f.len()) {
                return None;
            }
            millis = digits(f, f.len())? * 10_i64.pow(3 - f.len() as u32);
        }
    }
    if parts.next().is_some() || h > 23 || m > 59 || second > 59 {
        return None;
    }
    Some(((h * 60 + m) * 60 + second) * 1000 + millis)
}

fn parse(kind: &str, value: &str) -> Option<f64> {
    match kind {
        "number" | "range" => number(value, false),
        "date" => date(value).map(|n| n as f64),
        "month" => {
            let (y, m) = month(value)?;
            y.checked_sub(1970)?
                .checked_mul(12)?
                .checked_add(m - 1)
                .map(|n| n as f64)
        }
        "time" => time(value).map(|n| n as f64),
        "datetime-local" => {
            let (d, t) = value.split_once(['T', ' '])?;
            date(d)?.checked_add(time(t)?).map(|n| n as f64)
        }
        "week" => {
            let (y, w) = value.split_once("-W")?;
            let y = year(y)?;
            let w = digits(w, 2)?;
            // Monday=0; 1970-01-01 was Thursday.
            let jan1 = year_days(y)?;
            let weekday = jan1.checked_add(3)?.rem_euclid(7);
            let weeks = if weekday == 3 || (weekday == 2 && leap(y)) {
                53
            } else {
                52
            };
            if !(1..=weeks).contains(&w) {
                return None;
            }
            let monday = jan1.checked_add(3)?.checked_sub((weekday + 3) % 7)?;
            monday
                .checked_add((w - 1) * 7)?
                .checked_mul(DAY)
                .map(|n| n as f64)
        }
        _ => None,
    }
}

// Convert the finite parsed double's shortest decimal representation to a
// bounded exact integer/scale pair. Exponents cannot allocate unbounded powers:
// f64's decimal range bounds the canonical string to a few hundred digits.
fn decimal(value: f64) -> (BigInt, u32) {
    let text = value.to_string();
    let scale = text.split_once('.').map_or(0, |(_, f)| f.len() as u32);
    (
        text.replace('.', "").parse().expect("finite decimal"),
        scale,
    )
}

fn step_mismatch(value: f64, base: f64, step: f64, scale_factor: i64) -> bool {
    let (value, a) = decimal(value);
    let (base, b) = decimal(base);
    let (step, c) = decimal(step);
    let scale = a.max(b).max(c);
    let power = |n| BigInt::from(10).pow(n);
    let difference = value * power(scale - a) - base * power(scale - b);
    let step = step * scale_factor * power(scale - c);
    difference % step != BigInt::from(0)
}

fn step_parameters(kind: &str) -> (f64, i64, f64) {
    match kind {
        "date" => (1.0, DAY, 0.0),
        "week" => (1.0, DAY * 7, -259_200_000.0),
        "time" | "datetime-local" => (60.0, 1000, 0.0),
        _ => (1.0, 1, 0.0),
    }
}

fn floor_div(n: BigInt, d: &BigInt) -> BigInt {
    let q = &n / d;
    if n < BigInt::from(0) && &n % d != BigInt::from(0) {
        q - 1
    } else {
        q
    }
}

// At most 29 iterations across the entire i64 millisecond calendar domain.
// No timezone, locale, JavaScript callbacks or authored-size allocations.
fn civil_date(days: i64) -> Option<(i64, i64, i64)> {
    if days < year_days(1)? || days >= year_days(300_000_000)? {
        return None;
    }
    let (mut low, mut high) = (1, 300_000_000);
    while low + 1 < high {
        let mid = low + (high - low) / 2;
        if year_days(mid)? <= days {
            low = mid;
        } else {
            high = mid;
        }
    }
    let mut remaining = days - year_days(low)?;
    let lengths = [
        31,
        if leap(low) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    for (m, length) in lengths.into_iter().enumerate() {
        if remaining < length {
            return Some((low, m as i64 + 1, remaining + 1));
        }
        remaining -= length;
    }
    None
}

fn clock_string(milliseconds: i64) -> String {
    let t = milliseconds.rem_euclid(DAY);
    let mut text = format!("{:02}:{:02}", t / 3_600_000, t / 60_000 % 60);
    if t % 60_000 != 0 {
        text.push_str(&format!(":{:02}", t / 1000 % 60));
        if t % 1000 != 0 {
            text.push_str(&format!(".{:03}", t % 1000));
            text = text.trim_end_matches('0').into();
        }
    }
    text
}

fn serialize(kind: &str, value: f64) -> Option<String> {
    if !value.is_finite() {
        return None;
    }
    if matches!(kind, "number" | "range") {
        return Some(if value == 0.0 {
            "0".into()
        } else {
            value.to_string()
        });
    }
    if kind == "time" {
        return Some(clock_string(value.rem_euclid(DAY as f64).trunc() as i64));
    }
    if value < i64::MIN as f64 || value >= i64::MAX as f64 {
        return None;
    }
    let value = value.trunc() as i64;
    if kind == "month" {
        let y = 1970_i64.checked_add(value.div_euclid(12))?;
        return (y > 0).then(|| format!("{y:04}-{:02}", value.rem_euclid(12) + 1));
    }
    if kind == "time" {
        return Some(clock_string(value));
    }
    let days = value.div_euclid(DAY);
    let (y, m, d) = civil_date(days)?;
    match kind {
        "date" => Some(format!("{y:04}-{m:02}-{d:02}")),
        "datetime-local" => Some(format!("{y:04}-{m:02}-{d:02}T{}", clock_string(value))),
        "week" => {
            // The Thursday identifies the ISO week-year.
            let thursday = days.checked_add(3 - (days + 3).rem_euclid(7))?;
            let (y, _, _) = civil_date(thursday)?;
            let w = (thursday - year_days(y)?) / 7 + 1;
            Some(format!("{y:04}-W{w:02}"))
        }
        _ => None,
    }
}

fn snap_range(value: f64, min: f64, max: f64, base: f64, step: f64) -> Option<f64> {
    let values = [value, min, max, base, step].map(decimal);
    let scale = values.iter().map(|(_, scale)| *scale).max()?;
    let [value, min, max, base, step] =
        values.map(|(value, places)| value * BigInt::from(10).pow(scale - places));
    let floor_div = |n: BigInt, d: &BigInt| {
        let q = &n / d;
        if n < BigInt::from(0) && &n % d != BigInt::from(0) {
            q - 1
        } else {
            q
        }
    };
    let lower = -floor_div(&base - min, &step);
    let upper = floor_div(max - &base, &step);
    if lower > upper {
        return None;
    }
    // Round ties toward positive infinity without binary multiply/add error.
    let nearest = floor_div((value - &base) * 2 + &step, &(&step * 2));
    let result = base + nearest.max(lower).min(upper) * step;
    format!("{result}e-{scale}")
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}

impl DomRuntime {
    fn numeric_input_kind(&self, node: NodeId) -> Result<String, DomError> {
        if !self.is_element(node, HTML, "input") {
            return Ok(String::new());
        }
        Ok(self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("text")
            .to_ascii_lowercase())
    }

    pub fn control_value_as_number(&self, node: NodeId) -> Result<Option<f64>, DomError> {
        let kind = self.numeric_input_kind(node)?;
        Ok(parse(&kind, &self.control_value(node)?))
    }

    /// False means the API does not apply. NaN clears the current value using
    /// the ordinary native dirty-value/sanitization/journal path.
    pub fn set_control_value_as_number(
        &mut self,
        node: NodeId,
        value: f64,
    ) -> Result<bool, DomError> {
        let kind = self.numeric_input_kind(node)?;
        if !numeric_kind(&kind) {
            return Ok(false);
        }
        self.set_value(node, serialize(&kind, value).unwrap_or_default(), false)?;
        Ok(true)
    }

    pub fn control_supports_date(&self, node: NodeId) -> Result<bool, DomError> {
        Ok(matches!(
            self.numeric_input_kind(node)?.as_str(),
            "date" | "month" | "week" | "time"
        ))
    }

    pub fn control_value_as_date(&self, node: NodeId) -> Result<Option<f64>, DomError> {
        if !self.control_supports_date(node)? {
            return Ok(None);
        }
        let kind = self.numeric_input_kind(node)?;
        let value = self.control_value(node)?;
        Ok(if kind == "month" {
            date(&format!("{value}-01")).map(|n| n as f64)
        } else {
            parse(&kind, &value)
        })
    }

    pub fn set_control_value_as_date(
        &mut self,
        node: NodeId,
        value: f64,
    ) -> Result<bool, DomError> {
        if !self.control_supports_date(node)? {
            return Ok(false);
        }
        let kind = self.numeric_input_kind(node)?;
        let text = if kind == "month" {
            serialize("date", value).map(|s| s[..s.len() - 3].to_owned())
        } else {
            serialize(&kind, value)
        };
        self.set_value(node, text.unwrap_or_default(), false)?;
        Ok(true)
    }

    /// HTML stepping uses exact bounded decimal arithmetic. False requests an
    /// InvalidStateError; true includes specified no-op cases. Script stepping
    /// does not synthesize user input/change events.
    pub fn step_control(&mut self, node: NodeId, count: i32, down: bool) -> Result<bool, DomError> {
        let kind = self.numeric_input_kind(node)?;
        if !numeric_kind(&kind) {
            return Ok(false);
        }
        let attr = |name| self.attribute_in_null_namespace(node, name);
        let step_text = attr("step")?.unwrap_or("");
        if step_text.eq_ignore_ascii_case("any") {
            return Ok(false);
        }
        let (default, factor, default_base) = step_parameters(&kind);
        let step = number(step_text, false)
            .filter(|n| *n > 0.0)
            .unwrap_or(default);
        let authored_min = attr("min")?.and_then(|s| parse(&kind, s));
        let min = authored_min.or((kind == "range").then_some(0.0));
        let max = attr("max")?
            .and_then(|s| parse(&kind, s))
            .or((kind == "range").then_some(100.0));
        let base = authored_min
            .or_else(|| attr("value").ok().flatten().and_then(|s| parse(&kind, s)))
            .unwrap_or(default_base);
        let previous = parse(&kind, &self.control_value(node)?);
        let before = previous.unwrap_or(0.0);
        let values = [before, base, step, min.unwrap_or(0.0), max.unwrap_or(0.0)].map(decimal);
        let scale = values.iter().map(|(_, s)| *s).max().unwrap();
        let [before, base, step, lower, upper] =
            values.map(|(v, s)| v * BigInt::from(10).pow(scale - s));
        let step = step * factor;
        let first = -floor_div(&base - &lower, &step);
        let last = floor_div(&upper - &base, &step);
        if min.is_some() && max.is_some() && (lower > upper || first > last) {
            return Ok(true);
        }
        let difference = &before - &base;
        let count = if down { -(count as i64) } else { count as i64 };
        // Browser interoperability: snapping consumes the first requested step,
        // negative counts reverse direction, and an absent value has no prior
        // directional bound. This matches Blink's ApplyStep and pinned WPT's
        // input-stepdown-02; literal HTML algorithm wording differs here.
        let mut value = if count != 0 && &difference % &step != BigInt::from(0) {
            let index = floor_div(difference, &step) + if count < 0 { 0 } else { 1 };
            &base + (index + count - count.signum()) * &step
        } else {
            &before + &step * count
        };
        if min.is_some() && value < lower {
            value = &base + first * &step;
        }
        if max.is_some() && value > upper {
            value = &base + last * &step;
        }
        if previous.is_some() && ((count < 0 && value > before) || (count > 0 && value < before)) {
            return Ok(true);
        }
        if let Some(text) = format!("{value}e-{scale}")
            .parse::<f64>()
            .ok()
            .and_then(|v| serialize(&kind, v))
        {
            self.set_value(node, text, false)?;
        }
        Ok(true)
    }

    pub fn control_numeric_validity(&self, node: NodeId) -> Result<NumericValidity, DomError> {
        let mut result = NumericValidity::default();
        if !self.is_element(node, HTML, "input") {
            return Ok(result);
        }
        let kind = self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("text")
            .to_ascii_lowercase();
        if !numeric_kind(&kind) {
            return Ok(result);
        }
        let Some(value) = parse(&kind, &self.control_value(node)?) else {
            return Ok(result);
        };
        let attr = |name| self.attribute_in_null_namespace(node, name);
        let min = attr("min")?.and_then(|s| parse(&kind, s));
        let max = attr("max")?.and_then(|s| parse(&kind, s));
        if kind == "time" && min.zip(max).is_some_and(|(min, max)| min > max) {
            let outside = value < min.unwrap() && value > max.unwrap();
            result.range_underflow = outside;
            result.range_overflow = outside;
        } else {
            result.range_underflow = min.is_some_and(|min| value < min);
            result.range_overflow = max.is_some_and(|max| value > max);
        }
        let step = attr("step")?.unwrap_or("");
        if !step.eq_ignore_ascii_case("any") {
            let (default, scale, default_base) = step_parameters(&kind);
            let step = number(step, false).filter(|n| *n > 0.0).unwrap_or(default);
            let base = min
                .or_else(|| attr("value").ok().flatten().and_then(|s| parse(&kind, s)))
                .unwrap_or(default_base);
            result.step_mismatch = step_mismatch(value, base, step, scale);
        }
        Ok(result)
    }

    pub(super) fn sanitize_numeric_value(
        &self,
        node: NodeId,
        kind: &str,
        value: &str,
    ) -> Result<String, DomError> {
        if kind == "range" {
            let min = self
                .attribute_in_null_namespace(node, "min")?
                .and_then(|s| number(s, false))
                .unwrap_or(0.0);
            let max = self
                .attribute_in_null_namespace(node, "max")?
                .and_then(|s| number(s, false))
                .unwrap_or(100.0)
                .max(min);
            let mut value = number(value, true)
                .unwrap_or(min / 2.0 + max / 2.0)
                .clamp(min, max);
            let step = self
                .attribute_in_null_namespace(node, "step")?
                .unwrap_or("");
            if !step.eq_ignore_ascii_case("any") {
                let step = number(step, false).filter(|n| *n > 0.0).unwrap_or(1.0);
                let base = self
                    .attribute_in_null_namespace(node, "min")?
                    .and_then(|s| number(s, false))
                    .or_else(|| {
                        self.attribute_in_null_namespace(node, "value")
                            .ok()
                            .flatten()
                            .and_then(|s| number(s, false))
                    })
                    .unwrap_or(0.0);
                if let Some(rounded) = snap_range(value, min, max, base, step) {
                    value = rounded;
                }
            }
            return Ok(value.to_string());
        }
        if (kind == "number" && number(value, true).is_none()) || parse(kind, value).is_none() {
            return Ok(String::new());
        }
        if kind == "datetime-local" {
            let (date, clock) = value.split_once(['T', ' ']).expect("parsed datetime");
            let mut clock = clock.to_owned();
            if clock.contains('.') {
                clock = clock.trim_end_matches('0').trim_end_matches('.').into();
            }
            if clock.len() == 8 && clock.ends_with(":00") {
                clock.truncate(5);
            }
            return Ok(format!("{date}T{clock}"));
        }
        Ok(value.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_conversion_roundtrips_and_utc_calendar_boundaries() {
        for (kind, value) in [
            ("date", "0001-01-01"),
            ("date", "1900-03-01"),
            ("date", "2000-02-29"),
            ("date", "10000-01-01"),
            ("month", "1969-12"),
            ("week", "2020-W53"),
            ("week", "0001-W01"),
            ("week", "1970-W01"),
            ("datetime-local", "1969-12-31T23:59:59.999"),
            ("time", "12:30:01.001"),
            ("number", "0.3"),
        ] {
            assert_eq!(
                serialize(kind, parse(kind, value).unwrap()).as_deref(),
                Some(value),
                "{kind}"
            );
        }
        assert_eq!(
            serialize("time", 2.734_333_707_189_448e26).as_deref(),
            Some("10:54:10.944")
        );
        assert_eq!(serialize("time", -3_600_000.0).as_deref(), Some("23:00"));
        assert_eq!(serialize("date", f64::MAX), None);
        assert_eq!(serialize("datetime-local", f64::INFINITY), None);
        let mut dom = crate::html::parse("<input id=n type=month>")
            .into_runtime()
            .unwrap();
        let n = dom.find_by_id("n").unwrap();
        assert!(dom.set_control_value_as_number(n, -1.0).unwrap());
        assert_eq!(dom.control_value(n).unwrap(), "1969-12");
        assert_eq!(
            dom.control_value_as_date(n).unwrap(),
            Some(-2_678_400_000.0)
        );
        dom.set_control_value_as_date(n, 0.0).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "1970-01");
        dom.set_control_value_as_number(n, f64::NAN).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "");
        dom.set_attribute(n, "type", "text").unwrap();
        assert!(!dom.set_control_value_as_number(n, 1.0).unwrap());
        assert!(!dom.control_supports_date(n).unwrap());
    }

    #[test]
    fn stepping_uses_exact_lattice_and_directional_bounds() {
        let mut dom = crate::html::parse("<input id=n type=number step=0.1 min=0 value=0.2>")
            .into_runtime()
            .unwrap();
        let n = dom.find_by_id("n").unwrap();
        dom.step_control(n, 1, false).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "0.3");
        dom.set_value(n, "0.35", false).unwrap();
        dom.step_control(n, 20, false).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "2.3");
        dom.set_value(n, "0.35", false).unwrap();
        dom.step_control(n, 0, false).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "0.35");
        dom.step_control(n, -2, true).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "0.5");
        dom.step_control(n, -1, false).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "0.4");
        dom.set_attribute(n, "max", "0.45").unwrap();
        dom.step_control(n, i32::MAX, false).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "0.4");
        dom.set_value(n, "0.5", false).unwrap();
        dom.step_control(n, 1, false).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "0.5");
        dom.step_control(n, 1, true).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "0.4");
        dom.set_attribute(n, "step", "any").unwrap();
        assert!(!dom.step_control(n, 0, false).unwrap());
        dom.set_attribute(n, "step", "5e-324").unwrap();
        dom.set_value(n, "0", false).unwrap();
        dom.step_control(n, 1, false).unwrap();
        assert_eq!(dom.control_value_as_number(n).unwrap(), Some(5e-324));
        dom.set_attribute(n, "step", "1e308").unwrap();
        dom.remove_attribute_ns(n, None, "max").unwrap();
        dom.step_control(n, i32::MAX, false).unwrap();
        assert!(dom.control_value_as_number(n).unwrap().unwrap().is_finite());
        dom.set_attribute(n, "min", "7").unwrap();
        dom.set_attribute(n, "step", "1").unwrap();
        dom.set_value(n, "", false).unwrap();
        dom.step_control(n, 1, true).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "7");
        dom.set_value(n, "3", false).unwrap();
        dom.step_control(n, 1, true).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "3");
    }

    #[test]
    fn native_constraints_follow_values_attributes_and_type_transitions() {
        let mut dom = crate::html::parse("<input id=n type=number min=1 max=10 step=2 value=3><input id=t type=time min=21:00 max=06:00><input id=r type=range min=0 max=10 step=2>").into_runtime().unwrap();
        let n = dom.find_by_id("n").unwrap();
        let t = dom.find_by_id("t").unwrap();
        let r = dom.find_by_id("r").unwrap();
        assert!(dom.control_validity(n).unwrap());
        dom.set_value(n, "4", false).unwrap();
        assert!(dom.control_numeric_validity(n).unwrap().step_mismatch);
        dom.set_attribute(n, "step", "any").unwrap();
        assert!(dom.control_validity(n).unwrap());
        dom.set_value(n, "11", false).unwrap();
        assert!(dom.control_numeric_validity(n).unwrap().range_overflow);
        dom.set_attribute(n, "disabled", "").unwrap();
        assert!(!dom.control_will_validate(n).unwrap());
        assert!(dom.control_numeric_validity(n).unwrap().range_overflow);
        dom.set_value(n, "bad", false).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "");
        dom.set_attribute(n, "type", "text").unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "");
        dom.set_value(t, "12:00", false).unwrap();
        let v = dom.control_numeric_validity(t).unwrap();
        assert!(v.range_underflow && v.range_overflow);
        dom.set_value(t, "23:00", false).unwrap();
        assert!(dom.control_validity(t).unwrap());
        assert_eq!(dom.control_value(r).unwrap(), "6");
        dom.set_value(r, "11", false).unwrap();
        assert_eq!(dom.control_value(r).unwrap(), "10");
        dom.set_attribute(r, "max", "5").unwrap();
        assert_eq!(dom.control_value(r).unwrap(), "4");
        dom.set_attribute(n, "type", "datetime-local").unwrap();
        dom.set_value(n, "2024-02-29 12:30:00.000", false).unwrap();
        assert_eq!(dom.control_value(n).unwrap(), "2024-02-29T12:30");
    }
    #[test]
    fn calendars_and_float_grammar() {
        assert_eq!(parse("date", "1970-01-01"), Some(0.0));
        assert_eq!(parse("date", "1969-12-31"), Some(-86_400_000.0));
        for invalid in [
            "1900-02-29",
            "2000-02-30",
            "0000-01-01",
            "999-01-01",
            "2000-1-01",
        ] {
            assert_eq!(parse("date", invalid), None);
        }
        assert!(parse("date", "2000-02-29").is_some());
        assert!(parse("date", "10000-01-01").is_some());
        assert_eq!(parse("week", "1970-W01"), Some(-259_200_000.0));
        assert!(parse("week", "2020-W53").is_some());
        assert_eq!(parse("week", "2021-W53"), None);
        assert_eq!(parse("month", "1969-12"), Some(-1.0));
        assert_eq!(parse("time", "23:59:59.001"), Some(86_399_001.0));
        for invalid in ["24:00", "12:60", "12:00:60", "12:00:00.0001", "12:00Z"] {
            assert_eq!(parse("time", invalid), None);
        }
        for invalid in ["+1", " 1", "1.", "1e", "Infinity", "NaN", "0x10", "1e999"] {
            assert_eq!(number(invalid, true), None);
        }
        assert_eq!(number("-.8", true), Some(-0.8));
        assert_eq!(number(" +1.5tail", false), Some(1.5));
    }
    #[test]
    fn exact_decimal_step_arithmetic_is_bounded_by_finite_numbers() {
        assert_eq!(snap_range(0.6, 0.0, 1.0, 0.0, 0.1), Some(0.6));
        assert_eq!(snap_range(0.6, 0.1, 1.0, 0.1, 0.2), Some(0.7));
        assert_eq!(snap_range(-0.6, -1.0, 0.0, 0.0, 0.2), Some(-0.6));
        assert_eq!(snap_range(-0.5, -1.0, 0.0, 0.0, 0.2), Some(-0.4));
        assert_eq!(snap_range(0.5, 0.1, 0.9, 0.0, 1.0), None);
        assert!(!step_mismatch(3.6, 0.0, 0.003, 1));
        assert!(!step_mismatch(-12345678.9, 0.0, 1e-12, 1));
        assert!(step_mismatch(17.0, 0.0, 3e-15, 1));
        assert!(!step_mismatch(0.3, 0.1, 0.1, 1));
        assert!(step_mismatch(0.35, 0.1, 0.1, 1));
        assert!(!step_mismatch(f64::MAX, 0.0, 1.0, 1));
        assert!(!step_mismatch(5e-324, 0.0, 5e-324, 1));
    }
}
