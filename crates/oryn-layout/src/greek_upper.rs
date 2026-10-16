//! Greek uppercase tailoring over the original paragraph's character context.
use std::collections::BTreeMap;
use unicode_normalization::char::decompose_canonical;

#[derive(Default)]
struct Marks {
    accent: bool,
    diaeresis: bool,
    iotas: usize,
}
impl Marks {
    fn add(&mut self, ch: char) -> bool {
        match ch {
            '\u{300}' | '\u{301}' | '\u{302}' | '\u{303}' | '\u{311}' | '\u{342}' => {
                self.accent = true
            }
            '\u{308}' => self.diaeresis = true,
            '\u{344}' => {
                self.diaeresis = true;
                self.accent = true;
            }
            '\u{345}' => self.iotas += 1,
            '\u{304}' | '\u{306}' | '\u{313}' | '\u{314}' | '\u{343}' => {}
            _ => return false,
        }
        true
    }
}

/// Replacement strings are indexed by original scalar position. A base and its
/// consumed diacritics retain separate source identities in the caller.
pub(super) fn replacements_with_allocation<
    C,
    A: oryn_dom::projection_allocation::ProjectionAllocation,
>(
    chars: &[char],
    before_cased: &[bool],
    after_cased: &[bool],
    eligible: Option<&[bool]>,
    allocation: &mut A,
    checkpoint: &mut impl FnMut() -> Result<(), C>,
) -> Result<BTreeMap<usize, String>, crate::text_transform::CasePreparationError<C, A::Error>> {
    let mut replacements = BTreeMap::new();
    let eligible = |index: usize| eligible.is_none_or(|values| values[index]);
    // Accent origin determines whether an introduced diaeresis is precomposed.
    let mut previous_accent: Option<bool> = None;
    let mut index = 0;
    while index < chars.len() {
        checkpoint().map_err(crate::text_transform::CasePreparationError::Checkpoint)?;
        let ch = chars[index];
        if !eligible(index) || !matches!(ch as u32, 0x370..=0x3ff | 0x1f00..=0x1fff | 0x2126) {
            previous_accent = None;
            index += 1;
            continue;
        }
        let scratch_start = allocation.scratch_mark();
        let mut decomposed = Vec::new();
        let mut failed = None;
        decompose_canonical(ch, |part| {
            if failed.is_none()
                && let Err(error) =
                    crate::text_transform::case_push(&mut decomposed, part, allocation, checkpoint)
            {
                failed = Some(error);
            }
        });
        if let Some(error) = failed {
            return Err(error);
        }
        let scratch_end = allocation.scratch_mark();
        let base = decomposed[0];
        let mut uppercase = base.to_uppercase();
        let mut upper = uppercase.next().unwrap();
        if uppercase.next().is_some()
            || !matches!(upper as u32, 0x370..=0x3ff)
            || !upper.is_alphabetic()
        {
            previous_accent = None;
            index += 1;
            drop(decomposed);
            allocation.release_scratch_range(scratch_start..scratch_end);
            continue;
        }
        let vowel = matches!(upper, 'Α' | 'Ε' | 'Η' | 'Ι' | 'Ο' | 'Υ' | 'Ω');
        let mut marks = Marks::default();
        let mut accepted = true;
        for &part in &decomposed[1..] {
            checkpoint().map_err(crate::text_transform::CasePreparationError::Checkpoint)?;
            if !marks.add(part) {
                accepted = false;
                break;
            }
        }
        if !accepted {
            previous_accent = None;
            index += 1;
            drop(decomposed);
            allocation.release_scratch_range(scratch_start..scratch_end);
            continue;
        }
        drop(decomposed);
        allocation.release_scratch_range(scratch_start..scratch_end);
        let precomposed_accent = marks.accent;
        let mut precomposed_diaeresis = marks.diaeresis;
        if matches!(upper, 'Ι' | 'Υ')
            && let Some(precomposed) = previous_accent
        {
            marks.diaeresis = true;
            precomposed_diaeresis |= precomposed;
        }
        let mut end = index + 1;
        while end < chars.len() && eligible(end) && marks.add(chars[end]) {
            checkpoint().map_err(crate::text_transform::CasePreparationError::Checkpoint)?;
            end += 1;
        }
        previous_accent = (vowel && marks.accent && !marks.diaeresis).then_some(precomposed_accent);
        let disjunctive_eta = upper == 'Η'
            && marks.accent
            && marks.iotas == 0
            && !before_cased[index]
            && !after_cased[end - 1];
        let mut trailing_tonos = false;
        if disjunctive_eta {
            if precomposed_accent {
                upper = 'Ή';
            } else {
                trailing_tonos = true;
            }
        } else if precomposed_diaeresis {
            match upper {
                'Ι' => {
                    upper = 'Ϊ';
                    marks.diaeresis = false;
                }
                'Υ' => {
                    upper = 'Ϋ';
                    marks.diaeresis = false;
                }
                _ => {}
            }
        }
        let mut rendered = String::new();
        let mut buffer = [0u8; 4];
        crate::text_transform::case_append(
            &mut rendered,
            upper.encode_utf8(&mut buffer),
            allocation,
            checkpoint,
        )?;
        if marks.diaeresis {
            crate::text_transform::case_append(&mut rendered, "\u{308}", allocation, checkpoint)?;
        }
        if trailing_tonos {
            crate::text_transform::case_append(&mut rendered, "\u{301}", allocation, checkpoint)?;
        }
        for _ in 0..marks.iotas {
            checkpoint().map_err(crate::text_transform::CasePreparationError::Checkpoint)?;
            crate::text_transform::case_append(&mut rendered, "Ι", allocation, checkpoint)?;
        }
        replacements.insert(index, rendered);
        for consumed in index + 1..end {
            checkpoint().map_err(crate::text_transform::CasePreparationError::Checkpoint)?;
            replacements.insert(consumed, String::new());
        }
        index = end;
    }
    Ok(replacements)
}
