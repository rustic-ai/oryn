//! CSS text transformations without changing native DOM text.
//!
//! Context is computed over the paragraph, including across text-node and style
//! boundaries. Callers retain a separate source range for each replacement.
use icu_properties::{
    CodePointMapData, CodePointSetData,
    props::{CanonicalCombiningClass, CaseIgnorable, Cased, SoftDotted},
};
use unicode_segmentation::UnicodeSegmentation;

pub(super) struct CaseContext {
    chars: Vec<char>,
    word_initial: Vec<bool>,
    initial_grapheme: Vec<bool>,
    greek_upper: std::collections::BTreeMap<usize, String>,
    before_cased: Vec<bool>,
    after_cased: Vec<bool>,
    after_i: Vec<bool>,
    after_soft_dotted: Vec<bool>,
    more_above: Vec<bool>,
    before_dot: Vec<bool>,
}

impl CaseContext {
    #[cfg(test)]
    pub fn new(text: &str) -> Self {
        Self::build(text, None)
    }

    pub fn with_greek_eligibility(text: &str, eligible: &[bool]) -> Self {
        Self::build(text, Some(eligible))
    }

    fn build(text: &str, greek_eligible: Option<&[bool]>) -> Self {
        let chars: Vec<_> = text.chars().collect();
        let count = chars.len();
        let mut context = Self {
            chars,
            greek_upper: std::collections::BTreeMap::new(),
            word_initial: vec![false; count],
            initial_grapheme: vec![false; count],
            before_cased: vec![false; count],
            after_cased: vec![false; count],
            after_i: vec![false; count],
            after_soft_dotted: vec![false; count],
            more_above: vec![false; count],
            before_dot: vec![false; count],
        };
        let cased = CodePointSetData::new::<Cased>();
        // Segment the complete paragraph so inline/style boundaries cannot
        // accidentally start a new capitalized word. Numbers consume the initial
        // typographic unit too; initial punctuation does not.
        let mut offset = 0;
        for word in text.split_word_bounds() {
            let mut found = false;
            for ch in word.chars() {
                if !found
                    && !unicode_normalization::char::is_combining_mark(ch)
                    && (ch.is_alphanumeric() || cased.contains(ch))
                {
                    context.word_initial[offset] = true;
                    found = true;
                }
                offset += 1;
            }
        }
        let mut first = 0;
        for grapheme in text.graphemes(true) {
            let end = first + grapheme.chars().count();
            if context.word_initial[first..end]
                .iter()
                .any(|&initial| initial)
            {
                context.initial_grapheme[first..end].fill(true);
            }
            first = end;
        }
        let ignorable = CodePointSetData::new::<CaseIgnorable>();
        let soft_dotted = CodePointSetData::new::<SoftDotted>();
        let combining = CodePointMapData::<CanonicalCombiningClass>::new();
        let (mut preceding_cased, mut preceding_i, mut preceding_soft) = (false, false, false);
        for (index, &ch) in context.chars.iter().enumerate() {
            context.before_cased[index] = preceding_cased;
            context.after_i[index] = preceding_i;
            context.after_soft_dotted[index] = preceding_soft;
            if !ignorable.contains(ch) {
                preceding_cased = cased.contains(ch);
            }
            let class = combining.get(ch).to_icu4c_value();
            if class == 0 || class == 230 {
                preceding_i = ch == 'I';
                preceding_soft = soft_dotted.contains(ch);
            }
        }
        let (mut following_cased, mut above, mut dot) = (false, false, false);
        for (index, &ch) in context.chars.iter().enumerate().rev() {
            context.after_cased[index] = following_cased;
            context.more_above[index] = above;
            context.before_dot[index] = dot;
            if !ignorable.contains(ch) {
                following_cased = cased.contains(ch);
            }
            let class = combining.get(ch).to_icu4c_value();
            if class == 0 {
                above = false;
            } else if class == 230 {
                above = true;
            }
            if ch == '\u{0307}' {
                dot = true;
            } else if class == 0 || class == 230 {
                dot = false;
            }
        }
        context.greek_upper = crate::greek_upper::replacements(
            &context.chars,
            &context.before_cased,
            &context.after_cased,
            greek_eligible,
        );
        context
    }

    pub fn convert(&self, index: usize, transform: &str, language: &str) -> String {
        let width = transform
            .split_ascii_whitespace()
            .any(|token| token == "full-width");
        let kana = transform
            .split_ascii_whitespace()
            .any(|token| token == "full-size-kana");
        self.convert_case(index, transform, language)
            .chars()
            .map(|ch| {
                let ch = if width { full_width(ch) } else { ch };
                if kana { full_size_kana(ch) } else { ch }
            })
            .collect()
    }

    fn convert_case(&self, index: usize, transform: &str, language: &str) -> String {
        let ch = self.chars[index];
        let turkic = language.eq_ignore_ascii_case("tr") || language.eq_ignore_ascii_case("az");
        let lithuanian = language.eq_ignore_ascii_case("lt");
        // Combined CSS values still select at most one case transformation.
        let uppercase = transform
            .split_ascii_whitespace()
            .any(|token| token == "uppercase");
        let lowercase = transform
            .split_ascii_whitespace()
            .any(|token| token == "lowercase");
        let capitalize = transform
            .split_ascii_whitespace()
            .any(|token| token == "capitalize");
        if capitalize {
            // Titlecasing affects the initial typographic unit, including its
            // dependent marks, but never recases the remainder of the word.
            if lithuanian
                && ch == '\u{0307}'
                && self.initial_grapheme[index]
                && self.after_soft_dotted[index]
            {
                return String::new();
            }
            let dutch_ij = language.eq_ignore_ascii_case("nl")
                && index > 0
                && matches!(ch, 'j' | 'J')
                && matches!(self.chars[index - 1], 'i' | 'I')
                && self.word_initial[index - 1];
            if self.word_initial[index] || dutch_ij {
                if turkic && ch == 'i' {
                    return "İ".into();
                }
                return titlecase(ch);
            }
            return ch.to_string();
        }
        if uppercase {
            if language.eq_ignore_ascii_case("el") {
                if let Some(rendered) = self.greek_upper.get(&index) {
                    return rendered.clone();
                }
            }
            if turkic && ch == 'i' {
                return "İ".into();
            }
            if lithuanian && ch == '\u{0307}' && self.after_soft_dotted[index] {
                return String::new();
            }
            return ch.to_uppercase().collect();
        }
        if lowercase {
            if ch == 'Σ' && self.before_cased[index] && !self.after_cased[index] {
                return "ς".into();
            }
            if turkic {
                match ch {
                    'İ' => return "i".into(),
                    'I' if !self.before_dot[index] => return "ı".into(),
                    '\u{0307}' if self.after_i[index] => return String::new(),
                    _ => {}
                }
            }
            if lithuanian {
                match ch {
                    'I' | 'J' | 'Į' if self.more_above[index] => {
                        let mut result: String = ch.to_lowercase().collect();
                        result.push('\u{0307}');
                        return result;
                    }
                    'Ì' => return "i\u{0307}\u{0300}".into(),
                    'Í' => return "i\u{0307}\u{0301}".into(),
                    'Ĩ' => return "i\u{0307}\u{0303}".into(),
                    _ => {}
                }
            }
            return ch.to_lowercase().collect();
        }
        ch.to_string()
    }
}

/// Unicode titlecase differs from uppercase for digraphs, case expansions,
/// Greek prosgegrammeni and Georgian Mkhedruli. Preserve the remainder of the
/// original word; only the initial typographic unit passes through this mapping.
fn titlecase(ch: char) -> String {
    let scalar = match ch as u32 {
        0x01c4..=0x01c6 => Some(0x01c5),
        0x01c7..=0x01c9 => Some(0x01c8),
        0x01ca..=0x01cc => Some(0x01cb),
        0x01f1..=0x01f3 => Some(0x01f2),
        0x10d0..=0x10fa | 0x10fd..=0x10ff => return ch.to_string(),
        0x1f80..=0x1f87 | 0x1f90..=0x1f97 | 0x1fa0..=0x1fa7 => Some(ch as u32 + 8),
        0x1f88..=0x1f8f | 0x1f98..=0x1f9f | 0x1fa8..=0x1faf => return ch.to_string(),
        0x1fb3 | 0x1fbc => Some(0x1fbc),
        0x1fc3 | 0x1fcc => Some(0x1fcc),
        0x1ff3 | 0x1ffc => Some(0x1ffc),
        _ => None,
    };
    if let Some(scalar) = scalar {
        return char::from_u32(scalar).unwrap().to_string();
    }
    match ch {
        '\u{1fb2}' => return "\u{1fba}\u{345}".into(),
        '\u{1fb4}' => return "\u{386}\u{345}".into(),
        '\u{1fb7}' => return "Α\u{342}\u{345}".into(),
        '\u{1fc2}' => return "\u{1fca}\u{345}".into(),
        '\u{1fc4}' => return "\u{389}\u{345}".into(),
        '\u{1fc7}' => return "Η\u{342}\u{345}".into(),
        '\u{1ff2}' => return "\u{1ffa}\u{345}".into(),
        '\u{1ff4}' => return "\u{38f}\u{345}".into(),
        '\u{1ff7}' => return "Ω\u{342}\u{345}".into(),
        _ => {}
    }
    let cased = CodePointSetData::new::<Cased>();
    let mut first_cased = false;
    let mut result = String::new();
    for upper in ch.to_uppercase() {
        if first_cased {
            result.extend(upper.to_lowercase());
        } else {
            result.push(upper);
            first_cased = cased.contains(upper);
        }
    }
    result
}

/// Unicode narrow decomposition mappings and the inverse wide mappings. This
/// deliberately does not apply general compatibility normalization: circled
/// letters, ligatures and compatibility ideographs retain their identity.
fn full_width(ch: char) -> char {
    let mapped = match ch as u32 {
        0x20 => 0x3000,
        0x21..=0x7e => ch as u32 + 0xfee0,
        0xa2 => 0xffe0,
        0xa3 => 0xffe1,
        0xac => 0xffe2,
        0xaf => 0xffe3,
        0xa6 => 0xffe4,
        0xa5 => 0xffe5,
        0x20a9 => 0xffe6,
        0x2985 => 0xff5f,
        0x2986 => 0xff60,
        0xff61..=0xff9f => {
            const KANA: [char; 63] = [
                '。', '「', '」', '、', '・', 'ヲ', 'ァ', 'ィ', 'ゥ', 'ェ', 'ォ', 'ャ', 'ュ', 'ョ',
                'ッ', 'ー', 'ア', 'イ', 'ウ', 'エ', 'オ', 'カ', 'キ', 'ク', 'ケ', 'コ', 'サ', 'シ',
                'ス', 'セ', 'ソ', 'タ', 'チ', 'ツ', 'テ', 'ト', 'ナ', 'ニ', 'ヌ', 'ネ', 'ノ', 'ハ',
                'ヒ', 'フ', 'ヘ', 'ホ', 'マ', 'ミ', 'ム', 'メ', 'モ', 'ヤ', 'ユ', 'ヨ', 'ラ', 'リ',
                'ル', 'レ', 'ロ', 'ワ', 'ン', '\u{3099}', '\u{309a}',
            ];
            return KANA[(ch as u32 - 0xff61) as usize];
        }
        0xffa0 => 0x3164,
        0xffa1..=0xffbe => ch as u32 - 0xffa1 + 0x3131,
        0xffc2..=0xffc7 => ch as u32 - 0xffc2 + 0x314f,
        0xffca..=0xffcf => ch as u32 - 0xffca + 0x3155,
        0xffd2..=0xffd7 => ch as u32 - 0xffd2 + 0x315b,
        0xffda..=0xffdc => ch as u32 - 0xffda + 0x3161,
        0xffe8 => 0x2502,
        0xffe9..=0xffec => ch as u32 - 0xffe9 + 0x2190,
        0xffed => 0x25a0,
        0xffee => 0x25cb,
        _ => return ch,
    };
    char::from_u32(mapped).expect("full-width mapping is a Unicode scalar")
}

/// CSS Text's small-to-normal kana mapping, including Small Kana Extension.
/// Size is independent of width: half-width small kana stay half-width here.
fn full_size_kana(ch: char) -> char {
    match ch {
        'ぁ' => 'あ',
        'ぃ' => 'い',
        'ぅ' => 'う',
        'ぇ' => 'え',
        'ぉ' => 'お',
        'ゕ' => 'か',
        'ゖ' => 'け',
        'っ' => 'つ',
        'ゃ' => 'や',
        'ゅ' => 'ゆ',
        'ょ' => 'よ',
        'ゎ' => 'わ',
        'ァ' => 'ア',
        'ィ' => 'イ',
        'ゥ' => 'ウ',
        'ェ' => 'エ',
        'ォ' => 'オ',
        'ヵ' => 'カ',
        'ヶ' => 'ケ',
        'ッ' => 'ツ',
        'ャ' => 'ヤ',
        'ュ' => 'ユ',
        'ョ' => 'ヨ',
        'ヮ' => 'ワ',
        'ㇰ' => 'ク',
        'ㇱ' => 'シ',
        'ㇲ' => 'ス',
        'ㇳ' => 'ト',
        'ㇴ' => 'ヌ',
        'ㇵ' => 'ハ',
        'ㇶ' => 'ヒ',
        'ㇷ' => 'フ',
        'ㇸ' => 'ヘ',
        'ㇹ' => 'ホ',
        'ㇺ' => 'ム',
        'ㇻ' => 'ラ',
        'ㇼ' => 'リ',
        'ㇽ' => 'ル',
        'ㇾ' => 'レ',
        'ㇿ' => 'ロ',
        'ｧ' => 'ｱ',
        'ｨ' => 'ｲ',
        'ｩ' => 'ｳ',
        'ｪ' => 'ｴ',
        'ｫ' => 'ｵ',
        'ｯ' => 'ﾂ',
        'ｬ' => 'ﾔ',
        'ｭ' => 'ﾕ',
        'ｮ' => 'ﾖ',
        '\u{1b132}' => 'こ',
        '\u{1b150}' => 'ゐ',
        '\u{1b151}' => 'ゑ',
        '\u{1b152}' => 'を',
        '\u{1b155}' => 'コ',
        '\u{1b164}' => 'ヰ',
        '\u{1b165}' => 'ヱ',
        '\u{1b166}' => 'ヲ',
        '\u{1b167}' => 'ン',
        _ => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn convert(text: &str, transform: &str, lang: &str) -> String {
        let context = CaseContext::new(text);
        (0..text.chars().count())
            .map(|index| context.convert(index, transform, lang))
            .collect()
    }
    #[test]
    fn lithuanian_capitalization_changes_only_initial_grapheme_marks() {
        assert_eq!(
            convert(
                "i\u{307}eva ji\u{307} j\u{323}\u{307}onas",
                "capitalize",
                "lt"
            ),
            "Ieva Ji\u{307} J\u{323}onas"
        );
        assert_eq!(
            convert("i\u{301}\u{307}eva", "capitalize", "lt"),
            "I\u{301}\u{307}eva"
        );
        assert_eq!(convert("i\u{307}eva", "capitalize", "en"), "I\u{307}eva");
        // An initial combining mark is not itself the first typographic letter.
        assert_eq!(convert("\u{345}alpha", "capitalize", "en"), "\u{345}Alpha");
        assert_eq!(convert("α\u{345}λφα", "capitalize", "el"), "Α\u{345}λφα");
    }

    #[test]
    fn greek_uppercase_tailoring_preserves_diaeresis_and_disjunctive_eta() {
        assert_eq!(
            convert("καλημέρα αύριο θεϊκό ευφυΐα Νεράιδα", "uppercase", "el"),
            "ΚΑΛΗΜΕΡΑ ΑΥΡΙΟ ΘΕΪΚΟ ΕΥΦΥΪΑ ΝΕΡΑΪΔΑ"
        );
        assert_eq!(
            convert("ήσουν ή εγώ ή εσύ", "uppercase", "EL"),
            "ΗΣΟΥΝ Ή ΕΓΩ Ή ΕΣΥ"
        );
        assert_eq!(
            convert("ήταν ('ή') ή'α", "uppercase", "el"),
            "ΗΤΑΝ ('Ή') Η'Α"
        );
        assert_eq!(convert("όμηρος", "capitalize", "el"), "Όμηρος");
        assert_eq!(convert("άι", "uppercase", "en"), "ΆΙ");
        assert_eq!(convert("άι", "uppercase", "el"), "ΑΪ");
    }

    #[test]
    fn greek_combining_forms_preserve_accent_origin_and_iota_expansions() {
        assert_eq!(convert("α\u{301}ι", "uppercase", "el"), "ΑΙ\u{308}");
        assert_eq!(
            convert("η\u{301} η\u{301}ταν", "uppercase", "el"),
            "Η\u{301} ΗΤΑΝ"
        );
        assert_eq!(
            convert("ι\u{308}\u{301} α\u{308}\u{301}ι", "uppercase", "el"),
            "Ι\u{308} Α\u{308}Ι"
        );
        assert_eq!(
            convert("ᾄ α\u{313}\u{301}\u{345}\u{345}", "uppercase", "el"),
            "ΑΙ ΑΙΙ"
        );
        assert_eq!(
            convert("ῥ α\u{304}\u{306} α\u{301}\u{323}", "uppercase", "el"),
            "Ρ Α Α\u{323}"
        );
        assert_eq!(convert("a\u{301} ά!ι", "uppercase", "el"), "A\u{301} Α!Ι");
    }

    #[test]
    fn capitalize_preserves_noninitial_case_and_skips_initial_punctuation() {
        assert_eq!(
            convert(
                "hELLO wORLD can't cancel·lar (\"word\") 3d",
                "capitalize",
                "en"
            ),
            "HELLO WORLD Can't Cancel·lar (\"Word\") 3d"
        );
        assert_eq!(
            convert("({[-–\"«'.<?!transform", "capitalize", "en"),
            "({[-–\"«'.<?!Transform"
        );
        assert_eq!(
            convert("e\u{301}lan 😀foo", "capitalize", "en"),
            "E\u{301}lan 😀Foo"
        );
        assert_eq!(
            convert("ijsland IJssel ijMUIDEN", "capitalize", "NL"),
            "IJsland IJssel IJMUIDEN"
        );
        assert_eq!(convert("ijsland", "capitalize", "en"), "Ijsland");
        assert_eq!(
            convert("istanbul ızmir", "capitalize", "tr"),
            "İstanbul Izmir"
        );
    }

    #[test]
    fn titlecase_keeps_digraph_and_expansion_forms_distinct_from_uppercase() {
        assert_eq!(
            convert("ǆungla Ǆungla ǉ ǌ ǳ ßeta ﬃsh ŉame և", "capitalize", ""),
            "ǅungla ǅungla ǈ ǋ ǲ Sseta Ffish ʼName Եւ"
        );
        assert_eq!(titlecase('ᾀ'), "ᾈ");
        assert_eq!(titlecase('ᾈ'), "ᾈ");
        assert_eq!(titlecase('ᾳ'), "ᾼ");
        assert_eq!(titlecase('ῃ'), "ῌ");
        assert_eq!(titlecase('ῳ'), "ῼ");
        assert_eq!(titlecase('ᾲ'), "Ὰ\u{345}");
        assert_eq!(titlecase('ᾷ'), "Α\u{342}\u{345}");
        assert_eq!(titlecase('ა'), "ა");
        assert_eq!(titlecase('ⴀ'), "Ⴀ");
        assert_eq!(
            convert("ßeta ﬃsh", "capitalize full-width", ""),
            "Ｓｓｅｔａ　Ｆｆｉｓｈ"
        );
    }

    #[test]
    fn width_mapping_preserves_unrelated_compatibility_characters() {
        assert_eq!(
            convert("a A!~ ¢£¬¯¦¥₩ ⦅⦆", "full-width", ""),
            "ａ　Ａ！～　￠￡￢￣￤￥￦　｟｠"
        );
        assert_eq!(
            convert("①ﬃÅé😀\u{a0}\t\n", "full-width", ""),
            "①ﬃÅé😀\u{a0}\t\n"
        );
        assert_eq!(
            convert("ｶﾞﾊﾟ｡｢｣､･ｰ", "full-width", ""),
            "カ\u{3099}ハ\u{309a}。「」、・ー"
        );
        assert_eq!(
            convert(
                "\u{ffa0}\u{ffa1}\u{ffbe}\u{ffc2}\u{ffc7}\u{ffca}\u{ffcf}\u{ffd2}\u{ffd7}\u{ffda}\u{ffdc}",
                "full-width",
                ""
            ),
            "\u{3164}ㄱㅎㅏㅔㅕㅚㅛㅠㅡㅣ"
        );
        assert_eq!(
            convert(
                "\u{ffe8}\u{ffe9}\u{ffea}\u{ffeb}\u{ffec}\u{ffed}\u{ffee}",
                "full-width",
                ""
            ),
            "│←↑→↓■○"
        );
        assert_eq!(
            convert("\u{ffbf}\u{ffc0}\u{ffc1}\u{ffc8}\u{ffc9}", "full-width", ""),
            "\u{ffbf}\u{ffc0}\u{ffc1}\u{ffc8}\u{ffc9}"
        );
    }

    #[test]
    fn kana_size_is_independent_of_width_and_includes_supplementary_kana() {
        assert_eq!(
            convert("ぁぃぅぇぉゕゖっゃゅょゎ", "full-size-kana", ""),
            "あいうえおかけつやゆよわ"
        );
        assert_eq!(
            convert("ァィゥェォヵヶッャュョヮ", "full-size-kana", ""),
            "アイウエオカケツヤユヨワ"
        );
        assert_eq!(
            convert("ㇰㇱㇲㇳㇴㇵㇶㇷㇸㇹㇺㇻㇼㇽㇾㇿ", "full-size-kana", ""),
            "クシストヌハヒフヘホムラリルレロ"
        );
        assert_eq!(convert("ｧｨｩｪｫｯｬｭｮ", "full-size-kana", ""), "ｱｲｳｴｵﾂﾔﾕﾖ");
        assert_eq!(
            convert(
                "\u{1b132}\u{1b150}\u{1b151}\u{1b152}\u{1b155}\u{1b164}\u{1b165}\u{1b166}\u{1b167}",
                "full-size-kana",
                ""
            ),
            "こゐゑをコヰヱヲン"
        );
        assert_eq!(
            convert("あアイｱガハ\u{309a}ー", "full-size-kana", ""),
            "あアイｱガハ\u{309a}ー"
        );
    }

    #[test]
    fn transforms_apply_case_then_width_then_kana_regardless_of_css_order() {
        let text = "ßi ｧㇰ";
        let expected = "ＳＳＩ　アク";
        assert_eq!(
            convert(text, "uppercase full-width full-size-kana", "de"),
            expected
        );
        assert_eq!(
            convert(text, "full-size-kana full-width uppercase", "de"),
            expected
        );
        assert_eq!(convert("İ", "lowercase full-width", "tr"), "ｉ");
    }

    #[test]
    fn full_case_expansions_and_final_sigma_context() {
        assert_eq!(convert("Straße ﬃ", "uppercase", "de"), "STRASSE FFI");
        assert_eq!(
            convert("ΟΣ ΟΣΑ Σ Σ\u{301} ΟΣ\u{301} ΟΣ'Α", "lowercase", "el"),
            "ος οσα σ σ\u{301} ος\u{301} οσ'α"
        );
        assert_eq!(convert("İ", "lowercase", "en"), "i\u{307}");
    }
    #[test]
    fn turkic_context_and_combining_class_blockers() {
        assert_eq!(convert("iı", "uppercase", "tr"), "İI");
        assert_eq!(
            convert(
                "Iİ I\u{307} I\u{323}\u{307} I\u{301}\u{307}",
                "lowercase",
                "az"
            ),
            "ıi i i\u{323} ı\u{301}\u{307}"
        );
    }
    #[test]
    fn lithuanian_keeps_lowercase_dots_and_removes_uppercase_dots() {
        assert_eq!(
            convert("I\u{301} J\u{323}\u{300} Į\u{301} ÌÍĨ", "lowercase", "lt"),
            "i\u{307}\u{301} j\u{307}\u{323}\u{300} į\u{307}\u{301} i\u{307}\u{300}i\u{307}\u{301}i\u{307}\u{303}"
        );
        assert_eq!(
            convert(
                "i\u{307} j\u{323}\u{307} i\u{301}\u{307}",
                "uppercase",
                "lt"
            ),
            "I J\u{323} I\u{301}\u{307}"
        );
    }
}

/// Accessibility preserves semantic kana/width characters, but speaks CSS case
/// conversion. Context and scalar offsets retain capitalization and final-sigma
/// behavior across inline text-node boundaries without mutating DOM text.
pub fn accessibility_case_fragment(
    text: &str,
    start: usize,
    end: usize,
    transform: &str,
    language: &str,
) -> String {
    let case = transform
        .split_ascii_whitespace()
        .find(|token| matches!(*token, "uppercase" | "lowercase" | "capitalize"))
        .unwrap_or("none");
    let count = text.chars().count();
    let start = start.min(count);
    let end = end.min(count).max(start);
    let context = CaseContext::with_greek_eligibility(text, &vec![case == "uppercase"; count]);
    (start..end)
        .map(|index| context.convert(index, case, language))
        .collect()
}

#[cfg(test)]
mod accessibility_tests {
    use super::*;
    #[test]
    fn speech_case_uses_paragraph_context_without_semantic_kana_conversion() {
        assert_eq!(
            accessibility_case_fragment("hello world", 3, 11, "capitalize", "en"),
            "lo World"
        );
        assert_eq!(
            accessibility_case_fragment("ΟΣ", 0, 2, "lowercase", "el"),
            "ος"
        );
        assert_eq!(
            accessibility_case_fragment("びょういん", 0, 5, "full-size-kana", "ja"),
            "びょういん"
        );
    }
}
