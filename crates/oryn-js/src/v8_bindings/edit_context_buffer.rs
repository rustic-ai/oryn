//! Native EditContext buffer; author updates and trusted input are distinct.
//! Stores UTF-16 units so replacement cannot corrupt a selected surrogate half.
#[derive(Default, Debug)]
pub(super) struct EditContextBuffer {
    text: Vec<u16>,
    pub selection_start: u32,
    pub selection_end: u32,
    pub character_bounds_start: u32,
    pub control_bounds: [f64; 4],
    pub selection_bounds: [f64; 4],
}

impl EditContextBuffer {
    pub fn new(text: Vec<u16>, start: u32, end: u32) -> Self {
        Self { text, selection_start: start, selection_end: end, ..Self::default() }
    }

    pub fn text(&self) -> &[u16] { &self.text }

    /// Convert the complete replacement before entry. Failure leaves buffer and
    /// selection unchanged, and author updateText does not publish textupdate.
    pub fn update_text(&mut self, start: u32, end: u32, replacement: &[u16]) -> Result<(), &'static str> {
        let (start, end) = (
            (start.min(end) as usize).min(self.text.len()),
            (start.max(end) as usize).min(self.text.len()),
        );
        let length = self.text.len().checked_sub(end - start)
            .and_then(|length| length.checked_add(replacement.len()))
            .filter(|length| *length <= u32::MAX as usize)
            .ok_or("EditContext text length")?;
        let mut next = Vec::new();
        next.try_reserve_exact(length).map_err(|_| "EditContext text allocation")?;
        next.extend_from_slice(&self.text[..start]);
        next.extend_from_slice(replacement);
        next.extend_from_slice(&self.text[end..]);
        self.text = next;
        Ok(())
    }

    pub fn update_selection(&mut self, start: u32, end: u32) {
        self.selection_start = start;
        self.selection_end = end;
    }

    pub fn input_range(&self, deletion: Option<bool>) -> (u32, u32) {
        let limit = self.text.len() as u32;
        let start = self.selection_start.min(self.selection_end).min(limit);
        let end = self.selection_start.max(self.selection_end).min(limit);
        if start != end { return (start, end); }
        match deletion {
            Some(true) if start > 0 => {
                let mut before = start - 1;
                if before > 0 && (0xdc00..=0xdfff).contains(&self.text[before as usize])
                    && (0xd800..=0xdbff).contains(&self.text[before as usize - 1]) { before -= 1; }
                (before, end)
            }
            Some(false) if end < limit => {
                let mut after = end + 1;
                if after < limit && (0xd800..=0xdbff).contains(&self.text[end as usize])
                    && (0xdc00..=0xdfff).contains(&self.text[after as usize]) { after += 1; }
                (start, after)
            }
            _ => (start, end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backwards_replacement_and_selection_follow_full_basics_fixture() {
        let mut context = EditContextBuffer::default();
        context.update_text(6, 0, &"abcdef".encode_utf16().collect::<Vec<_>>()).unwrap();
        context.update_text(2, 5, &"ghi".encode_utf16().collect::<Vec<_>>()).unwrap();
        assert_eq!(String::from_utf16(context.text()).unwrap(), "abghif");
        context.update_text(5, 2, &"jkl".encode_utf16().collect::<Vec<_>>()).unwrap();
        assert_eq!(String::from_utf16(context.text()).unwrap(), "abjklf");
        context.update_selection(3, 0);
        assert_eq!((context.selection_start, context.selection_end), (3, 0));
    }
    #[test]
    fn replacement_preserves_lone_surrogates_without_utf8_roundtrip() {
        let mut context = EditContextBuffer::new(vec![0x61, 0xd83d, 0xde00, 0x62], 0, 0);
        context.update_text(2, 3, &[0x63]).unwrap();
        assert_eq!(context.text(), &[0x61, 0xd83d, 0x63, 0x62]);
    }
}
