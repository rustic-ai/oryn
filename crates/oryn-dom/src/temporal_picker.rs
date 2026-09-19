//! Native picker candidates. Display labels and highlighting never mutate the
//! input value; commit rechecks current constraints against the candidate.
use super::*;
use serde::{Deserialize, Serialize};
const DAY: f64 = 86_400_000.;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalPickerItem {
    pub label: String,
    pub value: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalPicker {
    pub page: usize,
    pub session: u64,
    pub draft: String,
    pub clock_field: Option<u8>,
    pub kind: String,
    pub year: i64,
    pub month: i64,
    pub active: usize,
    pub title: String,
    pub headings: Vec<String>,
    pub columns: usize,
    pub items: Vec<TemporalPickerItem>,
}
impl DomRuntime {
    pub fn set_native_ui_locale(&mut self, locale: &str) -> Result<(), DomError> {
        if locale.is_empty()
            || locale.len() > 128
            || !locale.split('-').all(|part| {
                !part.is_empty()
                    && part.len() <= 8
                    && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
        {
            return Err(DomError::InvalidUiLocale);
        }
        let locale = locale.to_ascii_lowercase();
        if self.native_ui_locale.as_ref() != Some(&locale) {
            self.native_ui_locale = Some(locale);
            let document = self.document();
            self.record_mutation(Mutation::FormState { node: document });
        }
        Ok(())
    }
    pub fn native_ui_locale(&self) -> Option<&str> {
        self.native_ui_locale.as_deref()
    }

    pub fn input_datalist(&self, node: NodeId) -> Result<Option<NodeId>, DomError> {
        if !self.is_element(node, "http://www.w3.org/1999/xhtml", "input") {
            return Ok(None);
        }
        let kind = self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("text")
            .to_ascii_lowercase();
        if matches!(
            kind.as_str(),
            "hidden"
                | "password"
                | "checkbox"
                | "radio"
                | "file"
                | "submit"
                | "image"
                | "reset"
                | "button"
        ) {
            return Ok(None);
        }
        let Some(id) = self
            .attribute_in_null_namespace(node, "list")?
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let target = self.get_element_by_id(self.control_tree_root(node)?, id)?;
        Ok(target
            .filter(|target| self.is_element(*target, "http://www.w3.org/1999/xhtml", "datalist")))
    }
    pub fn range_datalist_fractions(&self, node: NodeId) -> Result<Vec<f64>, DomError> {
        if !self
            .attribute_in_null_namespace(node, "type")?
            .is_some_and(|kind| kind.eq_ignore_ascii_case("range"))
        {
            return Ok(Vec::new());
        }
        let Some(list) = self.input_datalist(node)? else {
            return Ok(Vec::new());
        };
        let number = |name: &str| {
            self.attribute_in_null_namespace(node, name)
                .ok()
                .flatten()
                .and_then(parse_html_floating_point)
        };
        let min = number("min").unwrap_or(0.);
        let max = number("max").unwrap_or(100.).max(min);
        let mut ticks = Vec::new();
        if min == max {
            return Ok(ticks);
        }
        for option in self.arena.descendants_inclusive(list)? {
            if ticks.len() >= 1024 {
                break;
            }
            if !self.is_element(option, "http://www.w3.org/1999/xhtml", "option")
                || self.option_disabled(option)?
            {
                continue;
            }
            let value = self.option_value(option)?;
            let Some(number) = parse_html_floating_point(&value) else {
                continue;
            };
            if !self.temporal_candidate_selectable(node, &value)? {
                continue;
            }
            let span = max - min;
            let fraction = if span.is_finite() {
                (number - min) / span
            } else {
                (number / 2. - min / 2.) / (max / 2. - min / 2.)
            };
            if fraction.is_finite() && (0.0..=1.0).contains(&fraction) && !ticks.contains(&fraction)
            {
                ticks.push(fraction);
            }
        }
        ticks.sort_by(f64::total_cmp);
        Ok(ticks)
    }
    pub fn has_native_input_picker(&self, node: NodeId) -> bool {
        self.is_temporal_control(node)
            || (self.is_element(node, "http://www.w3.org/1999/xhtml", "input")
                && (self
                    .attribute_in_null_namespace(node, "type")
                    .ok()
                    .flatten()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("color"))
                    || (self.is_editable_text_control(node)
                        && self
                            .attribute_in_null_namespace(node, "type")
                            .ok()
                            .flatten()
                            .is_none_or(|kind| !kind.eq_ignore_ascii_case("password"))
                        && self.input_datalist(node).ok().flatten().is_some())))
    }
    pub fn temporal_language(&self, node: NodeId) -> Result<String, DomError> {
        Ok(self
            .element_language(node)?
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| self.native_ui_locale.clone().unwrap_or_default()))
    }
    pub fn temporal_candidate_selectable(
        &self,
        node: NodeId,
        value: &str,
    ) -> Result<bool, DomError> {
        let kind = self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("text")
            .to_ascii_lowercase();
        if kind == "color" {
            return Ok(color::parse_color(value).is_ok());
        }
        if !numeric_controls::numeric_kind(&kind) {
            return Ok(self.input_datalist(node)?.is_some());
        }
        if numeric_controls::parse(&kind, value).is_none() {
            return Ok(false);
        }
        let flags = self.control_numeric_validity_for_value(node, value)?;
        Ok(!flags.range_underflow && !flags.range_overflow && !flags.step_mismatch)
    }
    pub fn temporal_picker_snapshot(
        &self,
        node: NodeId,
    ) -> Result<Option<TemporalPicker>, DomError> {
        if !self.has_native_input_picker(node)
            || !self
                .top_layer()
                .any(|entry| entry.node == node && entry.kind == TopLayerKind::TemporalPicker)
        {
            return Ok(None);
        }
        let Some(state) = self.arena.get(node)?.temporal_picker.as_ref() else {
            return Ok(None);
        };
        let mut state = state.clone();
        if state.kind == "datalist" {
            let query = self.control_value(node)?;
            if query != state.draft {
                state.draft = query;
                state.page = 0;
                state.active = 0;
            }
        }
        self.update_temporal_picker_title(node, &mut state)?;
        state.items = self.temporal_picker_items(node, &state)?;
        state.active = state.active.min(state.items.len().saturating_sub(1));
        Ok(Some(state))
    }
    pub fn open_temporal_picker(
        &mut self,
        node: NodeId,
        today_millis: f64,
    ) -> Result<bool, DomError> {
        if !self.has_native_input_picker(node)
            || !self.is_connected(node)?
            || self.control_disabled(node)?
            || self.is_inert(node)?
            || (self.is_editable_text_control(node)
                && self
                    .attribute_in_null_namespace(node, "readonly")?
                    .is_some())
        {
            return Ok(false);
        }
        let mut kind = self
            .attribute_in_null_namespace(node, "type")?
            .unwrap_or("text")
            .to_ascii_lowercase();
        if !self.is_temporal_control(node) && kind != "color" {
            kind = "datalist".into();
        }
        let value = self.control_value(node)?;
        let seed = if value.is_empty() {
            self.attribute_in_null_namespace(node, "min")?
                .filter(|value| numeric_controls::parse(&kind, value).is_some())
                .unwrap_or("")
        } else {
            &value
        };
        let date = if matches!(kind.as_str(), "date" | "datetime-local" | "month") {
            seed.get(..7)
                .and_then(|v| v.split_once('-'))
                .and_then(|(y, m)| Some((y.parse::<i64>().ok()?, m.parse::<i64>().ok()?)))
        } else if kind == "week" {
            seed.split_once('-')
                .and_then(|(y, _)| Some((y.parse::<i64>().ok()?, 1)))
        } else {
            None
        };
        let (year, month) = date
            .filter(|(y, m)| *y > 0 && (1..=12).contains(m))
            .unwrap_or_else(|| {
                numeric_controls::civil_date((today_millis / DAY).floor() as i64)
                    .map(|(y, m, _)| (y, m))
                    .unwrap_or((1970, 1))
            });
        let mut picker = TemporalPicker {
            page: 0,
            session: self.mutation_cursor().0,
            draft: if value.is_empty() {
                seed.to_string()
            } else {
                value.clone()
            },
            clock_field: if kind == "time" || kind == "color" {
                Some(0)
            } else {
                None
            },
            kind,
            year,
            month,
            active: 0,
            title: String::new(),
            headings: Vec::new(),
            columns: 7,
            items: Vec::new(),
        };
        self.update_temporal_picker_title(node, &mut picker)?;
        picker.items = self.temporal_picker_items(node, &picker)?;
        picker.active = picker
            .items
            .iter()
            .position(|item| item.value == value)
            .or_else(|| picker.items.iter().position(|item| item.enabled))
            .unwrap_or(0);
        let old = self
            .top_layer()
            .filter(|entry| {
                matches!(
                    entry.kind,
                    TopLayerKind::SelectPicker | TopLayerKind::TemporalPicker
                )
            })
            .map(|entry| entry.node)
            .collect::<Vec<_>>();
        for old in old {
            self.remove_from_top_layer(old);
        }
        self.arena.get_mut(node)?.temporal_picker = Some(picker);
        self.add_to_top_layer(node, TopLayerKind::TemporalPicker)
    }
    fn update_temporal_picker_title(
        &self,
        node: NodeId,
        picker: &mut TemporalPicker,
    ) -> Result<(), DomError> {
        let lang = self.temporal_language(node)?;
        let profile = self.control_locale(node)?;
        let months = if lang.starts_with("fr") {
            [
                "janvier",
                "février",
                "mars",
                "avril",
                "mai",
                "juin",
                "juillet",
                "août",
                "septembre",
                "octobre",
                "novembre",
                "décembre",
            ]
        } else if lang.starts_with("de") {
            [
                "Januar",
                "Februar",
                "März",
                "April",
                "Mai",
                "Juni",
                "Juli",
                "August",
                "September",
                "Oktober",
                "November",
                "Dezember",
            ]
        } else {
            [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ]
        };
        picker.title = if picker.kind == "color" {
            format!(
                "{} — {}",
                match picker.clock_field.unwrap_or(0) {
                    0 => "Red",
                    1 => "Green",
                    2 => "Blue",
                    _ => "Alpha",
                },
                picker.draft
            )
        } else if picker.kind == "datalist" {
            "Suggestions".into()
        } else if let Some(field) = picker.clock_field {
            format!(
                "{} — {}",
                match field {
                    0 =>
                        if lang.starts_with("fr") {
                            "Heure"
                        } else if lang.starts_with("de") {
                            "Stunde"
                        } else {
                            "Hour"
                        },
                    1 => "Minute",
                    _ =>
                        if lang.starts_with("fr") {
                            "Seconde"
                        } else if lang.starts_with("de") {
                            "Sekunde"
                        } else {
                            "Second"
                        },
                },
                picker.draft
            )
        } else {
            match picker.kind.as_str() {
                "time" => {
                    if lang.starts_with("fr") {
                        "Heure".into()
                    } else if lang.starts_with("de") {
                        "Uhrzeit".into()
                    } else {
                        "Time".into()
                    }
                }
                "month" | "week" => picker.year.to_string(),
                _ => format!(
                    "{} {}",
                    profile.map_or(months[(picker.month - 1) as usize], |profile| profile
                        .months[(picker.month - 1) as usize]
                        .as_str()),
                    picker.year
                ),
            }
        };
        picker.columns = if picker.kind == "color" {
            16
        } else if picker.kind == "datalist" {
            1
        } else if picker.clock_field.is_some() {
            6
        } else if picker.kind == "date" || picker.kind == "datetime-local" {
            7
        } else {
            4
        };
        picker.headings = if picker.columns == 7
            && let Some(profile) = profile
        {
            (0..7)
                .map(|index| profile.weekdays[(index + profile.first_weekday as usize) % 7].clone())
                .collect()
        } else if picker.columns == 7 {
            let names = if lang.starts_with("fr") {
                ["lu", "ma", "me", "je", "ve", "sa", "di"]
            } else if lang.starts_with("de") {
                ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"]
            } else if lang.starts_with("en-us") {
                ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"]
            } else {
                ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
            };
            names.into_iter().map(str::to_string).collect()
        } else {
            Vec::new()
        };
        if let Some(profile) = profile {
            picker.title = profile.number(&picker.title);
        }
        Ok(())
    }
    fn temporal_picker_items(
        &self,
        node: NodeId,
        picker: &TemporalPicker,
    ) -> Result<Vec<TemporalPickerItem>, DomError> {
        let current = self.control_value(node)?;
        let fallback = self.attribute_in_null_namespace(node, "min")?.unwrap_or("");
        let clock_source = if current.is_empty() {
            fallback
        } else {
            &current
        };
        let mut candidates = Vec::new();
        if picker.kind == "color" {
            for channel in 0..=255 {
                candidates.push((
                    channel.to_string(),
                    self.color_picker_value(
                        node,
                        &picker.draft,
                        picker.clock_field.unwrap_or(0),
                        channel,
                    )?,
                ));
            }
        } else if picker.kind == "datalist" {
            if let Some(list) = self.input_datalist(node)? {
                let kind = self
                    .attribute_in_null_namespace(node, "type")?
                    .unwrap_or("text")
                    .to_ascii_lowercase();
                let multiple_email = kind == "email"
                    && self
                        .attribute_in_null_namespace(node, "multiple")?
                        .is_some();
                let (prefix, query) = if multiple_email {
                    current
                        .rsplit_once(',')
                        .map_or((None, current.as_str()), |(prefix, query)| {
                            (Some(prefix), query)
                        })
                } else {
                    (None, current.as_str())
                };
                let query = query
                    .trim_matches(['\t', '\n', '\x0c', '\r', ' '])
                    .to_lowercase();
                let mut seen = 0usize;
                for option in self.arena.descendants_inclusive(list)? {
                    if !self.is_element(option, "http://www.w3.org/1999/xhtml", "option")
                        || self.option_disabled(option)?
                    {
                        continue;
                    }
                    let value = self.option_value(option)?;
                    let label = self.option_label(option)?;
                    if (kind == "email" && !form_controls::valid_email_address(&value))
                        || (kind == "url" && url::Url::parse(&value).is_err())
                    {
                        continue;
                    }
                    if value.is_empty()
                        || (!query.is_empty()
                            && !value.to_lowercase().contains(&query)
                            && !label.to_lowercase().contains(&query))
                    {
                        continue;
                    }
                    if seen >= picker.page.saturating_mul(64) && candidates.len() < 64 {
                        let value =
                            prefix.map_or(value.clone(), |prefix| format!("{prefix},{value}"));
                        candidates.push((label, value));
                    }
                    seen += 1;
                }
            }
        } else if let Some(field) = picker.clock_field {
            let clock = picker
                .draft
                .split_once('T')
                .map(|(_, time)| time)
                .unwrap_or(if picker.kind == "time" {
                    &picker.draft
                } else {
                    "00:00"
                });
            let mut parts = clock.split(':').map(str::to_string).collect::<Vec<_>>();
            while parts.len() < 2 {
                parts.push("00".into());
            }
            if parts[0].is_empty() {
                parts[0] = "00".into();
            }
            if field == 2 && parts.len() < 3 {
                parts.push("00".into());
            }
            for value in 0..if field == 0 { 24 } else { 60 } {
                let mut next = parts.clone();
                let fraction = if field == 2 {
                    next.get(2)
                        .and_then(|part| part.split_once('.'))
                        .map(|(_, fraction)| format!(".{fraction}"))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                next[field as usize] = format!("{value:02}{fraction}");
                let clock = next.join(":");
                let value = if picker.kind == "datetime-local" {
                    format!(
                        "{}T{clock}",
                        picker.draft.split('T').next().unwrap_or("1970-01-01")
                    )
                } else {
                    clock
                };
                candidates.push((
                    format!(
                        "{:02}",
                        value
                            .split('T')
                            .next_back()
                            .unwrap_or(&value)
                            .split(':')
                            .nth(field as usize)
                            .unwrap_or("00")
                    ),
                    value,
                ));
            }
        } else {
            match picker.kind.as_str() {
                "month" => {
                    for month in 1..=12 {
                        candidates.push((
                            format!("{month:02}"),
                            format!("{:04}-{month:02}", picker.year),
                        ));
                    }
                }
                "week" => {
                    for week in 1..=53 {
                        let value = format!("{:04}-W{week:02}", picker.year);
                        if numeric_controls::parse("week", &value).is_some() {
                            candidates.push((format!("W{week:02}"), value));
                        }
                    }
                }
                "time" => return Ok(Vec::new()),
                _ => {
                    let first = numeric_controls::date(&format!(
                        "{:04}-{:02}-01",
                        picker.year, picker.month
                    ));
                    if let Some(first) = first {
                        let first = first as f64;
                        let first_weekday = self.control_locale(node)?.map_or(
                            if self.temporal_language(node)?.starts_with("en-us") {
                                0
                            } else {
                                1
                            },
                            |profile| profile.first_weekday,
                        );
                        let offset =
                            ((first / DAY) as i64 + 4 - i64::from(first_weekday)).rem_euclid(7);
                        let suffix = clock_source
                            .split_once('T')
                            .map(|(_, t)| t)
                            .unwrap_or("00:00");
                        for day in 0..42 {
                            if let Some(date) = numeric_controls::serialize(
                                "date",
                                first + ((day - offset) as f64) * DAY,
                            ) {
                                let label = date.rsplit('-').next().unwrap().to_string();
                                let value = if picker.kind == "datetime-local" {
                                    format!("{date}T{suffix}")
                                } else {
                                    date
                                };
                                candidates.push((label, value));
                            }
                        }
                    }
                }
            }
        }
        let last = self.temporal_picker_last_clock_field(node, picker)?;
        candidates
            .into_iter()
            .map(|(label, value)| {
                let enabled = if picker.clock_field.is_some_and(|field| field < last) {
                    true
                } else if picker.kind == "datetime-local" && picker.clock_field.is_none() {
                    let date = value.split('T').next().unwrap_or("");
                    let min = self
                        .attribute_in_null_namespace(node, "min")?
                        .and_then(|v| v.split_once('T'))
                        .map(|(date, _)| date);
                    let max = self
                        .attribute_in_null_namespace(node, "max")?
                        .and_then(|v| v.split_once('T'))
                        .map(|(date, _)| date);
                    min.is_none_or(|min| date >= min) && max.is_none_or(|max| date <= max)
                } else {
                    self.temporal_candidate_selectable(node, &value)?
                };
                let label = self
                    .control_locale(node)?
                    .map_or(label.clone(), |profile| profile.number(&label));
                Ok(TemporalPickerItem {
                    enabled,
                    label,
                    value,
                })
            })
            .collect()
    }
    fn temporal_picker_last_clock_field(
        &self,
        node: NodeId,
        picker: &TemporalPicker,
    ) -> Result<u8, DomError> {
        if picker.kind == "color" {
            return Ok(if self.color_input_configuration(node)?.1 {
                3
            } else {
                2
            });
        }
        let step = self
            .attribute_in_null_namespace(node, "step")?
            .unwrap_or("60");
        let seconds = step.eq_ignore_ascii_case("any")
            || step
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite() && *value > 0.)
                .is_some_and(|step| step.rem_euclid(60.) != 0.);
        Ok(if seconds || picker.draft.matches(':').count() > 1 {
            2
        } else {
            1
        })
    }
    pub fn choose_temporal_picker_item(
        &mut self,
        node: NodeId,
        index: usize,
    ) -> Result<Option<String>, DomError> {
        let Some(mut picker) = self.temporal_picker_snapshot(node)? else {
            return Ok(None);
        };
        let Some(item) = picker.items.get(index).filter(|item| item.enabled) else {
            return Ok(None);
        };
        let value = item.value.clone();
        if picker.kind == "datetime-local" && picker.clock_field.is_none() {
            picker.draft = value;
            picker.clock_field = Some(0);
        } else if picker.clock_field.is_some_and(|field| {
            field
                < self
                    .temporal_picker_last_clock_field(node, &picker)
                    .unwrap_or(1)
        }) {
            picker.draft = value;
            picker.clock_field = picker.clock_field.map(|field| field + 1);
        } else {
            return Ok(Some(value));
        }
        self.update_temporal_picker_title(node, &mut picker)?;
        picker.items = self.temporal_picker_items(node, &picker)?;
        picker.active = picker
            .items
            .iter()
            .position(|item| item.value == picker.draft)
            .unwrap_or(0);
        self.arena.get_mut(node)?.temporal_picker = Some(picker);
        self.record_mutation(Mutation::FormState { node });
        Ok(None)
    }
    pub fn scroll_temporal_picker(&mut self, node: NodeId, rows: i32) -> Result<(), DomError> {
        let Some(mut picker) = self.temporal_picker_snapshot(node)? else {
            return Ok(());
        };
        let target = picker.active as i64 + i64::from(rows) * picker.columns as i64;
        picker.active = target.clamp(0, picker.items.len().saturating_sub(1) as i64) as usize;
        self.arena.get_mut(node)?.temporal_picker = Some(picker);
        self.record_mutation(Mutation::FormState { node });
        Ok(())
    }
    pub fn move_temporal_picker(
        &mut self,
        node: NodeId,
        delta: i32,
        page: bool,
    ) -> Result<(), DomError> {
        let Some(mut picker) = self.temporal_picker_snapshot(node)? else {
            return Ok(());
        };
        if page {
            if picker.kind == "datalist" {
                let old = picker.page;
                picker.page = if delta < 0 {
                    picker.page.saturating_sub(delta.unsigned_abs() as usize)
                } else {
                    picker.page.saturating_add(delta as usize)
                };
                if self.temporal_picker_items(node, &picker)?.is_empty() {
                    picker.page = old;
                }
            } else if let Some(field) = picker.clock_field {
                let last = self.temporal_picker_last_clock_field(node, &picker)?;
                picker.clock_field =
                    Some((i32::from(field) + delta).clamp(0, i32::from(last)) as u8);
            } else if matches!(picker.kind.as_str(), "month" | "week") {
                picker.year = picker
                    .year
                    .saturating_add(i64::from(delta))
                    .clamp(1, 999999);
            } else {
                let total = (picker.year - 1) * 12 + picker.month - 1 + i64::from(delta);
                let total = total.clamp(0, 999999 * 12 - 1);
                picker.year = total / 12 + 1;
                picker.month = total % 12 + 1;
            }
            self.update_temporal_picker_title(node, &mut picker)?;
            picker.items = self.temporal_picker_items(node, &picker)?;
            picker.active = picker
                .items
                .iter()
                .position(|item| item.enabled)
                .unwrap_or(0);
        } else if !picker.items.is_empty() {
            picker.active = (picker.active as i64 + i64::from(delta))
                .rem_euclid(picker.items.len() as i64) as usize;
        }
        self.arena.get_mut(node)?.temporal_picker = Some(picker);
        self.record_mutation(Mutation::FormState { node });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_candidates_are_constraint_bound_and_focus_reset_retires_native_state() {
        let mut dom=html::parse("<input id=d type=date lang=fr value=2024-02-28 min=2024-02-01 max=2024-02-29><input id=other>").into_runtime().unwrap();
        let d = dom.find_by_id("d").unwrap();
        let other = dom.find_by_id("other").unwrap();
        dom.focus(Some(d)).unwrap();
        dom.open_temporal_picker(d, 0.).unwrap();
        let picker = dom.temporal_picker_snapshot(d).unwrap().unwrap();
        assert_eq!(picker.title, "février 2024");
        assert_eq!(picker.items.len(), 42);
        assert!(
            picker
                .items
                .iter()
                .any(|item| item.value == "2024-02-29" && item.enabled)
        );
        assert!(
            picker
                .items
                .iter()
                .any(|item| item.value == "2024-03-01" && !item.enabled)
        );
        dom.set_attribute(d, "max", "2024-02-28").unwrap();
        assert!(!dom.temporal_candidate_selectable(d, "2024-02-29").unwrap());
        dom.move_temporal_picker(d, 1, true).unwrap();
        assert_eq!(dom.control_value(d).unwrap(), "2024-02-28");
        dom.focus(Some(other)).unwrap();
        assert!(dom.temporal_picker_snapshot(d).unwrap().is_none());
        dom.open_temporal_picker(d, 0.).unwrap();
        dom.reset_control_value(d, "2024-02-28".into()).unwrap();
        assert!(dom.temporal_picker_snapshot(d).unwrap().is_none());
    }
    #[test]
    fn empty_clock_picker_preserves_minimum_minutes_and_wrapped_range() {
        let mut dom = html::parse("<input id=t type=time min=23:30 max=00:30 step=1800>")
            .into_runtime()
            .unwrap();
        let t = dom.find_by_id("t").unwrap();
        dom.open_temporal_picker(t, 0.).unwrap();
        let picker = dom.temporal_picker_snapshot(t).unwrap().unwrap();
        assert!(
            picker
                .items
                .iter()
                .any(|item| item.value == "23:30" && item.enabled)
        );
        assert!(
            picker
                .items
                .iter()
                .any(|item| item.value == "00:30" && item.enabled)
        );
        dom.choose_temporal_picker_item(t, 12).unwrap();
        let minutes = dom.temporal_picker_snapshot(t).unwrap().unwrap();
        assert!(minutes.items.iter().all(|item| !item.enabled));
    }
    #[test]
    fn native_picker_families_share_exact_candidate_validation() {
        for (kind, value) in [
            ("month", "2024-02"),
            ("week", "2020-W53"),
            ("time", "12:30"),
            ("datetime-local", "2024-02-29T12:30"),
        ] {
            let mut dom = html::parse(&format!("<input id=d type={kind} value='{value}'>"))
                .into_runtime()
                .unwrap();
            let d = dom.find_by_id("d").unwrap();
            dom.open_temporal_picker(d, 0.).unwrap();
            let picker = dom.temporal_picker_snapshot(d).unwrap().unwrap();
            assert_eq!(picker.items[picker.active].value, value);
            assert!(picker.items[picker.active].enabled);
        }
    }
}

#[cfg(test)]
mod additional_tests {
    use super::*;
    #[test]
    fn native_locale_fallback_and_explicit_language_keep_canonical_values() {
        let mut dom = html::parse("<input id=d type=date value=2024-02-29>")
            .into_runtime()
            .unwrap();
        let d = dom.find_by_id("d").unwrap();
        dom.set_native_ui_locale("de-DE").unwrap();
        assert_eq!(dom.temporal_display_value(d).unwrap(), "29.02.2024");
        assert_eq!(dom.control_value(d).unwrap(), "2024-02-29");
        dom.set_attribute(d, "lang", "en-US").unwrap();
        assert_eq!(dom.temporal_display_value(d).unwrap(), "02/29/2024");
        assert!(dom.set_native_ui_locale("invalid/locale").is_err());
    }
    #[test]
    fn datalist_filters_native_value_and_label_and_list_is_inapplicable_to_password() {
        let mut dom=html::parse("<input id=t list=l value=AL><input id=p type=password list=l><datalist id=l><option value=alpha><option value=beta label=ALIAS><option value=alpine disabled><option value=other></datalist>").into_runtime().unwrap();
        let t = dom.find_by_id("t").unwrap();
        assert!(
            dom.input_datalist(dom.find_by_id("p").unwrap())
                .unwrap()
                .is_none()
        );
        dom.open_temporal_picker(t, 0.).unwrap();
        let picker = dom.temporal_picker_snapshot(t).unwrap().unwrap();
        assert_eq!(
            picker
                .items
                .iter()
                .map(|item| item.value.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
        dom.scroll_temporal_picker(t, 999).unwrap();
        assert_eq!(dom.temporal_picker_snapshot(t).unwrap().unwrap().active, 1);
        dom.set_value(t, "other", false).unwrap();
        let picker = dom.temporal_picker_snapshot(t).unwrap().unwrap();
        assert_eq!(picker.page, 0);
        assert_eq!(picker.active, 0);
        assert_eq!(picker.items[0].value, "other");
    }
}

#[cfg(test)]
mod range_tests {
    use super::*;
    #[test]
    fn range_ticks_use_only_valid_distinct_current_datalist_options() {
        let mut dom=html::parse("<input id=r type=range min=0 max=10 step=2 list=l><datalist id=l><option value=0><option value=4><option value=4><option value=3><option value=12><option value=6 disabled><option value=10></datalist>").into_runtime().unwrap();
        let r = dom.find_by_id("r").unwrap();
        assert_eq!(dom.range_datalist_fractions(r).unwrap(), vec![0., 0.4, 1.]);
        dom.set_attribute(r, "max", "4").unwrap();
        assert_eq!(dom.range_datalist_fractions(r).unwrap(), vec![0., 1.]);
    }
}

#[cfg(test)]
mod email_suggestions_tests {
    use super::*;
    #[test]
    fn multiple_email_suggestions_replace_only_last_token_and_reject_invalid_options() {
        let mut dom=html::parse("<input id=e type=email multiple list=l value='first@example.test, se'><datalist id=l><option value='second@example.test'><option value='not an address'><option value='other@example.test'></datalist>").into_runtime().unwrap();
        let e = dom.find_by_id("e").unwrap();
        dom.open_temporal_picker(e, 0.).unwrap();
        let picker = dom.temporal_picker_snapshot(e).unwrap().unwrap();
        assert_eq!(picker.items.len(), 1);
        assert_eq!(
            picker.items[0].value,
            "first@example.test,second@example.test"
        );
    }
}

#[cfg(test)]
mod datalist_candidate_tests {

    #[test]
    fn missing_type_datalist_uses_text_candidate_rules() {
        let dom =
            crate::html::parse("<input id=t list=l><datalist id=l><option value=alpha></datalist>")
                .into_runtime()
                .unwrap();
        let node = dom.find_by_id("t").unwrap();
        assert!(dom.temporal_candidate_selectable(node, "alpha").unwrap());
    }
}
