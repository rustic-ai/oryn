//! Native segmented date/time editing. Canonical ISO field order keeps native
//! text, caret offsets and sanitized public values independent of author code.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Year,
    Month,
    Day,
    Week,
    Hour,
    Minute,
    Second,
    Millisecond,
}
impl Field {
    fn width(self) -> usize {
        match self {
            Self::Year => 4,
            Self::Millisecond => 3,
            _ => 2,
        }
    }
    fn placeholder(self) -> &'static str {
        match self {
            Self::Year => "yyyy",
            Self::Month => "mm",
            Self::Day => "dd",
            Self::Week => "ww",
            Self::Hour => "hh",
            Self::Minute => "mm",
            Self::Second => "ss",
            Self::Millisecond => "sss",
        }
    }
    fn bounds(self) -> (u32, u32) {
        match self {
            Self::Year => (1, 999999),
            Self::Month => (1, 12),
            Self::Day => (1, 31),
            Self::Week => (1, 53),
            Self::Hour => (0, 23),
            Self::Minute | Self::Second => (0, 59),
            Self::Millisecond => (0, 999),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemporalEdit {
    kind: String,
    locale: String,
    // A locale profile contains large fixed arrays. TemporalEdit is optional
    // on every DOM Node; absent temporal state must not reserve those arrays.
    profile: Option<Box<ControlLocale>>,
    fields: Vec<(Field, Option<u32>)>,
    active: usize,
    digits: usize,
    advanced: bool,
}
#[derive(Clone, Copy, Debug)]
pub enum TemporalAction {
    Digit(u8),
    Move(i8),
    Separator(char),
    First,
    Last,
    Increment(i32),
    Clear,
    ClearAll,
}
#[derive(Clone, Copy, Debug)]
pub struct TemporalEditOutcome {
    pub changed: bool,
    pub moved: bool,
    pub selection: (u32, u32),
}
impl TemporalEdit {
    fn new(kind: &str, value: &str, seconds: bool, milliseconds: bool) -> Self {
        use Field::*;
        let mut fields = match kind {
            "date" => vec![Year, Month, Day],
            "month" => vec![Year, Month],
            "week" => vec![Year, Week],
            "time" => vec![Hour, Minute],
            _ => vec![Year, Month, Day, Hour, Minute],
        };
        if matches!(kind, "time" | "datetime-local") && seconds {
            fields.push(Second);
            if milliseconds {
                fields.push(Millisecond);
            }
        }
        let values = value
            .split(['-', ':', 'T', ' ', '.'])
            .map(|part| part.trim_start_matches('W').parse::<u32>().ok())
            .collect::<Vec<_>>();
        let fields = fields
            .into_iter()
            .enumerate()
            .map(|(i, field)| {
                let mut parsed = values.get(i).copied().flatten();
                if matches!(field, Second | Millisecond)
                    && i >= values.len()
                    && values.iter().all(Option::is_some)
                {
                    parsed = Some(0);
                }
                if field == Millisecond {
                    let digits = value
                        .rsplit_once('.')
                        .map_or(3, |(_, fraction)| fraction.len())
                        .min(3);
                    parsed =
                        parsed.map(|value| value.saturating_mul(10u32.pow((3 - digits) as u32)));
                }
                (field, parsed)
            })
            .collect::<Vec<_>>();
        let active = if value.is_empty() {
            0
        } else {
            fields.len().saturating_sub(1)
        };
        Self {
            kind: kind.into(),
            locale: String::new(),
            profile: None,
            fields,
            active,
            digits: 0,
            advanced: false,
        }
    }
    fn separator(&self, index: usize) -> &'static str {
        match self.fields[index].0 {
            Field::Year => "",
            Field::Month | Field::Day => "-",
            Field::Week => "-W",
            Field::Hour => {
                if index == 0 {
                    ""
                } else {
                    "T"
                }
            }
            Field::Minute | Field::Second => ":",
            Field::Millisecond => ".",
        }
    }
    fn display_indices(&self) -> Vec<usize> {
        let mut result = (0..self.fields.len()).collect::<Vec<_>>();
        let date = matches!(self.kind.as_str(), "date" | "datetime-local");
        let month = self.kind == "month";
        if let Some(profile) = &self.profile {
            if date {
                for (index, field) in profile.date_order.iter().enumerate() {
                    result[index] = *field as usize;
                }
            } else if month {
                let order = profile
                    .date_order
                    .iter()
                    .filter(|field| **field != 2)
                    .copied()
                    .collect::<Vec<_>>();
                result[..2].copy_from_slice(&[order[0] as usize, order[1] as usize]);
            }
            return result;
        }
        if date && self.locale.starts_with("en-us") {
            result[..3].copy_from_slice(&[1, 2, 0]);
        } else if date
            && (self.locale.starts_with("en-gb")
                || self.locale.starts_with("fr")
                || self.locale.starts_with("de"))
        {
            result[..3].copy_from_slice(&[2, 1, 0]);
        } else if month
            && (self.locale.starts_with("en-us")
                || self.locale.starts_with("en-gb")
                || self.locale.starts_with("fr")
                || self.locale.starts_with("de"))
        {
            result[..2].copy_from_slice(&[1, 0]);
        }
        result
    }
    fn display_separator(&self, position: usize, index: usize) -> &str {
        if position == 0 {
            return "";
        }
        let date_fields = if matches!(self.kind.as_str(), "date" | "datetime-local") {
            3
        } else if self.kind == "month" {
            2
        } else {
            0
        };
        if position < date_fields
            && let Some(profile) = &self.profile
        {
            return &profile.separators[position - 1];
        }
        if position < date_fields && !self.locale.is_empty() {
            return if self.locale.starts_with("de") {
                "."
            } else {
                "/"
            };
        }
        self.separator(index)
    }
    fn next_field(&self, delta: isize) -> Option<usize> {
        let order = self.display_indices();
        let current = order.iter().position(|index| *index == self.active)?;
        let next = current as isize + delta;
        if next < 0 {
            return None;
        }
        order.get(next as usize).copied()
    }
    fn render(&self, placeholders: bool) -> String {
        if !placeholders && self.fields.iter().all(|(_, value)| value.is_none()) {
            return String::new();
        }
        let mut result = String::new();
        let order = if placeholders {
            self.display_indices()
        } else {
            (0..self.fields.len()).collect()
        };
        for (position, index) in order.into_iter().enumerate() {
            let (field, value) = self.fields[index];
            result.push_str(if placeholders {
                self.display_separator(position, index)
            } else {
                self.separator(index)
            });
            match value {
                Some(value) => {
                    let text = format!("{value:0width$}", width = field.width());
                    result.push_str(&if placeholders {
                        self.profile
                            .as_ref()
                            .map_or(text.clone(), |profile| profile.number(&text))
                    } else {
                        text
                    })
                }
                None if placeholders => result.push_str(field.placeholder()),
                None => {}
            }
        }
        result
    }
    fn selection(&self) -> (u32, u32) {
        let mut start = 0;
        for (position, index) in self.display_indices().into_iter().enumerate() {
            let (field, value) = self.fields[index];
            start += dom_string::len(self.display_separator(position, index));
            let text = value.map_or_else(
                || field.placeholder().to_string(),
                |v| format!("{v:0width$}", width = field.width()),
            );
            let text = self
                .profile
                .as_ref()
                .map_or(text.clone(), |profile| profile.number(&text));
            let width = dom_string::len(&text);
            if index == self.active {
                return (start as u32, (start + width) as u32);
            }
            start += width;
        }
        (0, 0)
    }
    fn apply(&mut self, action: TemporalAction) -> bool {
        let previous = self.active;
        let (field, current) = self.fields[self.active];
        if !matches!(
            action,
            TemporalAction::Digit(_) | TemporalAction::Separator(_)
        ) {
            self.advanced = false;
        }
        match action {
            TemporalAction::Separator(separator) => {
                if self.advanced {
                    self.advanced = false;
                    return true;
                }
                if matches!(self.kind.as_str(), "time" | "datetime-local")
                    && self.active + 1 == self.fields.len()
                {
                    let next = match (field, separator) {
                        (Field::Minute, ':') => Some(Field::Second),
                        (Field::Second, '.') => Some(Field::Millisecond),
                        _ => None,
                    };
                    if let Some(field) = next {
                        self.fields.push((field, None));
                        self.active += 1;
                        self.digits = 0;
                        return true;
                    }
                }
                if let Some(next) = self.next_field(1) {
                    self.active = next;
                }
                self.digits = 0;
            }
            TemporalAction::Move(delta) => {
                let Some(next) = self.next_field(delta as isize) else {
                    return false;
                };
                self.active = next;
                self.digits = 0;
                self.advanced = false;
            }
            TemporalAction::First => {
                self.active = self.display_indices()[0];
                self.digits = 0;
            }
            TemporalAction::Last => {
                self.active = *self.display_indices().last().unwrap();
                self.digits = 0;
            }
            TemporalAction::ClearAll => {
                for (_, value) in &mut self.fields {
                    *value = None;
                }
                self.active = self.display_indices()[0];
                self.digits = 0;
                self.advanced = false;
            }
            TemporalAction::Clear => {
                self.fields[self.active].1 = None;
                self.digits = 0;
            }
            TemporalAction::Digit(digit) => {
                self.advanced = false;
                if digit > 9 {
                    return false;
                }
                let (min, max) = field.bounds();
                let mut value = if self.digits == 0 {
                    u32::from(digit)
                } else {
                    current
                        .unwrap_or(0)
                        .saturating_mul(10)
                        .saturating_add(u32::from(digit))
                };
                if value > max {
                    value = u32::from(digit);
                    self.digits = 0;
                }
                self.fields[self.active].1 = Some(value);
                self.digits += 1;
                if self.digits >= field.width() || (value >= min && value.saturating_mul(10) > max)
                {
                    if let Some(next) = self.next_field(1) {
                        self.active = next;
                        self.advanced = true;
                    }
                    self.digits = 0;
                }
            }
            TemporalAction::Increment(delta) => {
                let (min, max) = field.bounds();
                let value = current.map_or(if delta < 0 { max } else { min }, |value| {
                    ((i64::from(value) - i64::from(min) + i64::from(delta))
                        .rem_euclid(i64::from(max - min + 1))
                        + i64::from(min)) as u32
                });
                self.fields[self.active].1 = Some(value);
                self.digits = 0;
            }
        }
        previous != self.active || !matches!(action, TemporalAction::Move(_))
    }
}
impl DomRuntime {
    pub fn is_temporal_control(&self, node: NodeId) -> bool {
        self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
            && self
                .attribute_in_null_namespace(node, "type")
                .ok()
                .flatten()
                .is_some_and(|kind| {
                    matches!(
                        kind.to_ascii_lowercase().as_str(),
                        "date" | "month" | "week" | "time" | "datetime-local"
                    )
                })
    }
    fn temporal_model(&self, node: NodeId, value: &str) -> Result<TemporalEdit, DomError> {
        let kind = self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("date")
            .to_ascii_lowercase();
        let step = self
            .attribute_in_null_namespace(node, "step")?
            .and_then(|value| {
                if value.eq_ignore_ascii_case("any") {
                    Some(0.001)
                } else {
                    value
                        .parse::<f64>()
                        .ok()
                        .filter(|step| step.is_finite() && *step > 0.)
                }
            })
            .unwrap_or(60.);
        let time = value.split_once('T').map_or(value, |(_, time)| time);
        let bounds = [
            self.attribute_in_null_namespace(node, "min")?,
            self.attribute_in_null_namespace(node, "max")?,
        ];
        let has_seconds = time.matches(':').count() > 1
            || bounds
                .iter()
                .flatten()
                .any(|value| value.matches(':').count() > 1);
        let fractional =
            value.contains('.') || bounds.iter().flatten().any(|value| value.contains('.'));
        let mut edit = TemporalEdit::new(
            &kind,
            value,
            has_seconds || step.rem_euclid(60.) != 0. || fractional,
            fractional || step.fract() != 0.,
        );
        edit.locale = self.temporal_language(node)?;
        edit.profile = self.control_locale(node)?.cloned().map(Box::new);
        if value.is_empty() {
            edit.active = edit.display_indices()[0];
        }
        Ok(edit)
    }
    fn current_temporal_model(&self, node: NodeId) -> Result<TemporalEdit, DomError> {
        if let Some(previous) = &self.arena.get(node)?.temporal_edit {
            let mut next = self.temporal_model(node, &previous.render(false))?;
            for (field, value) in &mut next.fields {
                if let Some((_, old)) = previous.fields.iter().find(|(old, _)| *old == *field) {
                    *value = *old;
                }
            }
            next.active = next
                .fields
                .iter()
                .position(|(field, _)| *field == previous.fields[previous.active].0)
                .unwrap_or(next.fields.len() - 1);
            next.digits = previous.digits;
            next.advanced = previous.advanced;
            Ok(next)
        } else {
            self.temporal_model(node, &self.control_value(node)?)
        }
    }
    pub fn temporal_display_value(&self, node: NodeId) -> Result<String, DomError> {
        Ok(self.current_temporal_model(node)?.render(true))
    }
    pub fn temporal_selection(&self, node: NodeId) -> Result<(u32, u32), DomError> {
        Ok(self.current_temporal_model(node)?.selection())
    }
    pub fn edit_temporal_control(
        &mut self,
        node: NodeId,
        action: TemporalAction,
    ) -> Result<Option<TemporalEditOutcome>, DomError> {
        if !self.is_temporal_control(node) {
            return Ok(None);
        }
        let previous = self.temporal_display_value(node)?;
        let mut edit = self.current_temporal_model(node)?;
        let raw_before = edit.render(false);
        let moved = edit.apply(action);
        let raw = edit.render(false);
        let changed = raw != raw_before;
        if changed {
            let sensitive = self.arena.get(node)?.sensitive_value;
            self.set_value(node, raw, sensitive)?;
            let record = self.arena.get_mut(node)?;
            record.user_edited_value = true;
            if record.pending_edit_value.is_none() {
                record.pending_edit_value = Some(previous);
            }
        }
        let selection = edit.selection();
        self.arena.get_mut(node)?.temporal_edit = Some(edit);
        self.record_mutation(Mutation::FormState { node });
        Ok(Some(TemporalEditOutcome {
            changed,
            moved,
            selection,
        }))
    }
    fn temporal_native_text_to_canonical(
        &self,
        node: NodeId,
        value: &str,
    ) -> Result<String, DomError> {
        let model = self.temporal_model(node, "")?;
        let normalized = model
            .profile
            .as_ref()
            .map_or(value.to_string(), |profile| profile.ascii(value));
        let value = normalized.as_str();
        let date_fields = if matches!(model.kind.as_str(), "date" | "datetime-local") {
            3
        } else if model.kind == "month" {
            2
        } else {
            return Ok(value.into());
        };
        let (date, time) = value
            .split_once('T')
            .map_or((value, None), |(d, t)| (d, Some(t)));
        let canonical = date.split('-').collect::<Vec<_>>();
        if (canonical.len() == date_fields
            && canonical[0].len() >= 4
            && canonical
                .iter()
                .all(|part| part.bytes().all(|b| b.is_ascii_digit())))
            || (!date.contains('/')
                && !date.contains('.')
                && !date.contains('-')
                && model.profile.is_none())
        {
            return Ok(value.into());
        }
        let localized = model.profile.as_ref().map_or(date.to_string(), |profile| {
            let mut date = date.to_string();
            for separator in &profile.separators {
                let separator = profile.ascii(separator);
                if !separator.is_empty() {
                    date = date.replace(&separator, "/");
                }
            }
            date
        });
        let parts = localized.split(['/', '.']).collect::<Vec<_>>();
        if parts.len() != date_fields {
            return Ok(value.into());
        }
        let mut canonical = vec![""; date_fields];
        for (position, index) in model
            .display_indices()
            .into_iter()
            .take(date_fields)
            .enumerate()
        {
            canonical[index] = parts[position];
        }
        let mut value = canonical.join("-");
        if let Some(time) = time {
            value.push('T');
            value.push_str(time);
        }
        Ok(value)
    }
    pub fn restore_control_edit_value(
        &mut self,
        node: NodeId,
        value: String,
        sensitive: bool,
    ) -> Result<(), DomError> {
        if !self.is_temporal_control(node) {
            return self.set_value(node, value, sensitive);
        }
        let canonical = self.temporal_native_text_to_canonical(node, &value)?;
        let edit = self.temporal_model(node, &canonical)?;
        self.set_value(node, edit.render(false), sensitive)?;
        self.arena.get_mut(node)?.temporal_edit = Some(edit);
        Ok(())
    }
    pub fn align_temporal_selection(&mut self, node: NodeId, start: u32) -> Result<(), DomError> {
        let mut edit = self.current_temporal_model(node)?;
        let active = (0..edit.fields.len())
            .find(|index| {
                let mut probe = edit.clone();
                probe.active = *index;
                let (a, b) = probe.selection();
                start >= a && start <= b
            })
            .unwrap_or(edit.active);
        if active != edit.active {
            edit.active = active;
            edit.digits = 0;
            edit.advanced = false;
        }
        self.arena.get_mut(node)?.temporal_edit = Some(edit);
        Ok(())
    }
    pub(super) fn replace_temporal_edit(
        &mut self,
        node: NodeId,
        start: u32,
        end: u32,
        replacement: &str,
    ) -> Result<u32, DomError> {
        let before = self.temporal_display_value(node)?;
        let mut units = dom_string::units(&before);
        if start > end || end as usize > units.len() {
            return Err(OpaqueRangeError::InvalidOffset.into());
        }
        units.splice(start as usize..end as usize, dom_string::units(replacement));
        let value = dom_string::from_units(&units);
        let canonical = self.temporal_native_text_to_canonical(node, &value)?;
        let mut edit = self.temporal_model(node, &canonical)?;
        // A complete ISO replacement is accepted as one native edit; partial
        // fields remain native badInput rather than becoming an authored value.
        let caret =
            (start as usize + dom_string::len(replacement)).min(dom_string::len(&value)) as u32;
        let caret = caret.min(edit.render(true).len() as u32);
        edit.active = (0..edit.fields.len())
            .find(|index| {
                let mut probe = edit.clone();
                probe.active = *index;
                let (a, b) = probe.selection();
                caret >= a && caret <= b
            })
            .unwrap_or(edit.fields.len() - 1);
        let raw = edit.render(false);
        let sensitive = self.arena.get(node)?.sensitive_value;
        self.set_value(node, raw, sensitive)?;
        let record = self.arena.get_mut(node)?;
        record.user_edited_value = true;
        record.temporal_edit = Some(edit);
        if record.pending_edit_value.is_none() {
            record.pending_edit_value = Some(before);
        }
        Ok(caret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_locale_profile_keeps_nodes_compact_and_clones_independent() {
        assert!(std::mem::size_of::<Option<TemporalEdit>>() <= 128);
        assert!(std::mem::size_of::<Node>() <= 1536);
        let mut edit = TemporalEdit::new("date", "2024-02-29", false, false);
        edit.profile = Some(Box::new(ControlLocale {
            months: std::array::from_fn(|i| format!("month{i}")),
            weekdays: std::array::from_fn(|i| format!("day{i}")),
            first_weekday: 0,
            date_order: [0, 1, 2],
            separators: ["-".into(), "-".into()],
            digits: std::array::from_fn(|i| char::from(b'0' + i as u8)),
        }));
        let mut cloned = edit.clone();
        assert_eq!(cloned, edit);
        cloned.profile.as_mut().unwrap().months[0] = "clone only".into();
        assert_eq!(edit.profile.as_ref().unwrap().months[0], "month0");
        assert_ne!(cloned, edit);
        assert_eq!(cloned.render(false), edit.render(false));
    }

    fn digits(dom: &mut DomRuntime, node: NodeId, value: &str) {
        for byte in value.bytes() {
            dom.edit_temporal_control(node, TemporalAction::Digit(byte - b'0'))
                .unwrap();
        }
    }
    #[test]
    fn segmented_calendar_and_clock_edits_share_native_validation() {
        for (kind, step, typed, expected) in [
            ("date", "1", "20240229", "2024-02-29"),
            ("month", "1", "202609", "2026-09"),
            ("week", "1", "202053", "2020-W53"),
            ("time", "0.001", "235959125", "23:59:59.125"),
            ("datetime-local", "60", "202402292359", "2024-02-29T23:59"),
        ] {
            let mut dom = html::parse(&format!("<input id=c type={kind} step={step} required>"))
                .into_runtime()
                .unwrap();
            let node = dom.find_by_id("c").unwrap();
            dom.edit_temporal_control(node, TemporalAction::Digit(2))
                .unwrap();
            assert!(dom.control_bad_input(node).unwrap());
            assert_eq!(dom.control_value(node).unwrap(), "");
            dom.reset_control_value(node, String::new()).unwrap();
            digits(&mut dom, node, typed);
            assert_eq!(dom.control_value(node).unwrap(), expected, "{kind}");
            assert!(!dom.control_bad_input(node).unwrap());
            assert!(dom.control_validity(node).unwrap());
            dom.edit_temporal_control(node, TemporalAction::Clear)
                .unwrap();
            assert!(dom.control_bad_input(node).unwrap());
            assert!(dom.control_value(node).unwrap().is_empty());
            let clone = dom.clone_node(node, false).unwrap();
            assert!(!dom.control_bad_input(clone).unwrap());
            dom.reset_control_value(node, String::new()).unwrap();
            assert!(!dom.control_bad_input(node).unwrap());
            assert!(dom.arena().get(node).unwrap().temporal_edit.is_none());
        }
    }
    #[test]
    fn clock_field_precision_follows_effective_step_without_erasing_partial_edits() {
        let mut dom = html::parse("<input id=t type=time step=90>")
            .into_runtime()
            .unwrap();
        let node = dom.find_by_id("t").unwrap();
        assert_eq!(dom.control_edit_value(node).unwrap(), "hh:mm:ss");
        dom.set_attribute(node, "step", "1.5").unwrap();
        assert_eq!(dom.control_edit_value(node).unwrap(), "hh:mm:ss.sss");
        dom.set_attribute(node, "step", "0").unwrap();
        assert_eq!(dom.control_edit_value(node).unwrap(), "hh:mm");
        dom.edit_temporal_control(node, TemporalAction::Digit(2))
            .unwrap();
        dom.set_attribute(node, "step", "1.5").unwrap();
        assert_eq!(dom.control_edit_value(node).unwrap(), "02:mm:ss.sss");
        assert!(dom.control_bad_input(node).unwrap());
    }
    #[test]
    fn invalid_calendar_dates_and_fractional_clock_values_are_not_normalized_into_valid_dates() {
        let mut dom =
            html::parse("<input id=d type=date><input id=t type=time value='12:34:56.5'>")
                .into_runtime()
                .unwrap();
        let d = dom.find_by_id("d").unwrap();
        let t = dom.find_by_id("t").unwrap();
        digits(&mut dom, d, "20230229");
        dom.set_attribute(d, "min", "2020-01-01").unwrap();
        assert!(dom.control_bad_input(d).unwrap());
        assert_eq!(dom.control_value(d).unwrap(), "");
        assert_eq!(dom.control_edit_value(d).unwrap(), "2023-02-29");
        assert_eq!(dom.control_edit_value(t).unwrap(), "12:34:56.500");
        let before = dom.control_edit_value(d).unwrap();
        dom.restore_control_edit_value(d, before.clone(), false)
            .unwrap();
        dom.arena_mut().get_mut(d).unwrap().user_edited_value = true;
        assert_eq!(dom.control_edit_value(d).unwrap(), before);
        assert!(dom.control_bad_input(d).unwrap());
        dom.set_value(d, "2024-02-29", false).unwrap();
        assert!(!dom.control_bad_input(d).unwrap());
        assert!(dom.control_validity(d).unwrap());
    }
}
