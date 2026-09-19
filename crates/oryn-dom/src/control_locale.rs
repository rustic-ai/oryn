//! Native locale data for Gregorian HTML controls. Public values stay ISO/ASCII.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ControlLocale {
    pub months: [String; 12],
    pub weekdays: [String; 7],
    pub first_weekday: u8,
    pub date_order: [u8; 3],
    pub separators: [String; 2],
    pub digits: [char; 10],
}
impl ControlLocale {
    pub fn valid(&self) -> bool {
        let mut order = self.date_order;
        order.sort();
        order == [0, 1, 2]
            && self.first_weekday < 7
            && self
                .months
                .iter()
                .chain(self.weekdays.iter())
                .all(|value| !value.is_empty() && value.len() <= 128)
            && self.separators.iter().all(|value| value.len() <= 32)
            && self.digits.iter().copied().collect::<BTreeSet<_>>().len() == 10
    }
    pub fn number(&self, value: &str) -> String {
        value
            .chars()
            .map(|c| {
                if c.is_ascii_digit() {
                    self.digits[c as usize - '0' as usize]
                } else {
                    c
                }
            })
            .collect()
    }
    pub fn ascii(&self, value: &str) -> String {
        value
            .chars()
            .filter(|c| !matches!(c, '\u{200e}' | '\u{200f}' | '\u{061c}'))
            .map(|c| {
                self.digits
                    .iter()
                    .position(|digit| *digit == c)
                    .map_or(c, |i| (b'0' + i as u8) as char)
            })
            .collect()
    }
}
impl DomRuntime {
    pub fn install_control_locale(
        &mut self,
        locale: &str,
        profile: ControlLocale,
    ) -> Result<(), DomError> {
        if locale.len() > 128
            || !profile.valid()
            || self.control_locales.len() >= 64 && !self.control_locales.contains_key(locale)
        {
            return Err(DomError::InvalidUiLocale);
        }
        self.control_locales
            .insert(locale.to_ascii_lowercase(), profile);
        let document = self.document();
        self.record_mutation(Mutation::FormState { node: document });
        Ok(())
    }
    pub fn control_locale(&self, node: NodeId) -> Result<Option<&ControlLocale>, DomError> {
        Ok(self.control_locales.get(&self.temporal_language(node)?))
    }
    pub fn missing_control_locales(&self) -> Vec<String> {
        if self.control_locales.len() >= 64 {
            return Vec::new();
        }
        let mut result = BTreeSet::new();
        for (node, _) in self.arena.iter() {
            if self.is_temporal_control(node)
                && let Ok(locale) = self.temporal_language(node)
                && !locale.is_empty()
                && !self.control_locales.contains_key(&locale)
                && result.len() < 64
            {
                result.insert(locale);
            }
        }
        result.into_iter().collect()
    }
    pub fn temporal_digit(&self, node: NodeId, key: &str) -> Result<Option<u8>, DomError> {
        let mut chars = key.chars();
        let Some(c) = chars.next() else {
            return Ok(None);
        };
        if chars.next().is_some() {
            return Ok(None);
        }
        Ok(if c.is_ascii_digit() {
            Some(c as u8 - b'0')
        } else {
            self.control_locale(node)?.and_then(|locale| {
                locale
                    .digits
                    .iter()
                    .position(|digit| *digit == c)
                    .map(|index| index as u8)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn arabic() -> ControlLocale {
        ControlLocale {
            months: std::array::from_fn(|index| format!("month{}", index + 1)),
            weekdays: std::array::from_fn(|index| format!("day{index}")),
            first_weekday: 6,
            date_order: [2, 1, 0],
            separators: ["/".into(), "/".into()],
            digits: ['٠', '١', '٢', '٣', '٤', '٥', '٦', '٧', '٨', '٩'],
        }
    }
    #[test]
    fn native_number_data_keeps_iso_values_and_utf16_selection() {
        let mut dom = html::parse("<input id=d type=date lang=ar value=2024-02-29>")
            .into_runtime()
            .unwrap();
        let d = dom.find_by_id("d").unwrap();
        dom.install_control_locale("ar", arabic()).unwrap();
        assert_eq!(dom.temporal_display_value(d).unwrap(), "٢٩/٠٢/٢٠٢٤");
        assert_eq!(dom.control_value(d).unwrap(), "2024-02-29");
        assert_eq!(dom.temporal_digit(d, "٧").unwrap(), Some(7));
        dom.edit_temporal_control(d, TemporalAction::First).unwrap();
        assert_eq!(dom.temporal_selection(d).unwrap(), (0, 2));
        dom.open_temporal_picker(d, 0.).unwrap();
        let picker = dom.temporal_picker_snapshot(d).unwrap().unwrap();
        assert_eq!(picker.headings[0], "day6");
        assert!(
            picker
                .items
                .iter()
                .any(|item| item.value == "2024-02-29" && item.label == "٢٩")
        );
    }
    #[test]
    fn malformed_or_oversized_locale_profiles_are_rejected() {
        let mut dom = DomRuntime::new();
        let mut profile = arabic();
        profile.date_order = [0, 0, 1];
        assert!(dom.install_control_locale("ar", profile).is_err());
        let mut profile = arabic();
        profile.months[0] = "x".repeat(129);
        assert!(dom.install_control_locale("ar", profile).is_err());
    }
}
