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
pub(super) fn replacements(
    chars: &[char],
    before_cased: &[bool],
    after_cased: &[bool],
    eligible: Option<&[bool]>,
) -> BTreeMap<usize, String> {
    let mut replacements = BTreeMap::new();
    let eligible = |index: usize| eligible.is_none_or(|values| values[index]);
    // Accent origin determines whether an introduced diaeresis is precomposed.
    let mut previous_accent: Option<bool> = None;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if !eligible(index) || !matches!(ch as u32, 0x370..=0x3ff | 0x1f00..=0x1fff | 0x2126) {
            previous_accent = None;
            index += 1;
            continue;
        }
        let mut decomposed = Vec::new();
        decompose_canonical(ch, |part| decomposed.push(part));
        let base = decomposed[0];
        let mut uppercase = base.to_uppercase();
        let mut upper = uppercase.next().unwrap();
        if uppercase.next().is_some()
            || !matches!(upper as u32, 0x370..=0x3ff)
            || !upper.is_alphabetic()
        {
            previous_accent = None;
            index += 1;
            continue;
        }
        let vowel = matches!(upper, 'Α' | 'Ε' | 'Η' | 'Ι' | 'Ο' | 'Υ' | 'Ω');
        let mut marks = Marks::default();
        if !decomposed[1..].iter().all(|&part| marks.add(part)) {
            previous_accent = None;
            index += 1;
            continue;
        }
        let precomposed_accent = marks.accent;
        let mut precomposed_diaeresis = marks.diaeresis;
        if matches!(upper, 'Ι' | 'Υ') {
            if let Some(precomposed) = previous_accent {
                marks.diaeresis = true;
                precomposed_diaeresis |= precomposed;
            }
        }
        let mut end = index + 1;
        while end < chars.len() && eligible(end) && marks.add(chars[end]) {
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
        let mut rendered = upper.to_string();
        if marks.diaeresis {
            rendered.push('\u{308}');
        }
        if trailing_tonos {
            rendered.push('\u{301}');
        }
        for _ in 0..marks.iotas {
            rendered.push('Ι');
        }
        replacements.insert(index, rendered);
        for consumed in index + 1..end {
            replacements.insert(consumed, String::new());
        }
        index = end;
    }
    replacements
}
