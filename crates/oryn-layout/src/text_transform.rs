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
        match Self::build_with_checkpoint(text, greek_eligible, &mut || {
            Ok::<_, std::convert::Infallible>(())
        }) {
            Ok(value) => value,
            Err(never) => match never {},
        }
    }
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
    match accessibility_case_fragment_with_checkpoint(
        text,
        start,
        end,
        transform,
        language,
        &mut || Ok::<_, std::convert::Infallible>(()),
    ) {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

/// The same case algorithm with caller-owned terminal cancellation checkpoints.
/// Cancellation never produces an empty or partially transformed speech value.

#[cfg(test)]
mod accessibility_tests {
    use super::*;
    #[test]
    fn checkpoint_case_conversion_preserves_multilingual_context_and_cancellation() {
        for (text, transform, language) in [
            ("hello world", "capitalize", "en"),
            ("ΟΣ ΟΣΑ", "lowercase", "el"),
            ("ή άι α\u{301}ι", "uppercase", "el"),
            ("I\u{307} İ", "lowercase", "tr"),
            ("I\u{301}", "lowercase", "lt"),
        ] {
            let count = text.chars().count();
            assert_eq!(
                accessibility_case_fragment_with_checkpoint(
                    text,
                    0,
                    count,
                    transform,
                    language,
                    &mut || Ok::<_, ()>(())
                )
                .unwrap(),
                accessibility_case_fragment(text, 0, count, transform, language)
            );
        }
        let text = "α".repeat(256);
        let mut visits = 0;
        let result = accessibility_case_fragment_with_checkpoint(
            &text,
            0,
            256,
            "uppercase",
            "el",
            &mut || {
                visits += 1;
                if visits == 70 { Err("retired") } else { Ok(()) }
            },
        );
        assert_eq!(result, Err("retired"));
        assert_eq!(visits, 70);
    }

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

#[derive(Debug)]
pub enum CasePreparationError<C, A> {
    Checkpoint(C),
    Allocation(A),
}
pub(super) fn case_push<T, C, A: oryn_dom::projection_allocation::ProjectionAllocation>(
    values: &mut Vec<T>,
    value: T,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<(), CasePreparationError<C, A::Error>> {
    check().map_err(CasePreparationError::Checkpoint)?;
    let needed = values.len().checked_add(1).ok_or_else(|| {
        CasePreparationError::Allocation(A::Error::from(
            oryn_dom::DomError::SemanticProjectionCapacity,
        ))
    })?;
    oryn_dom::projection_allocation::reserve_vector(values, needed, allocation)
        .map_err(CasePreparationError::Allocation)?;
    values.push(value);
    Ok(())
}
pub(super) fn case_append<C, A: oryn_dom::projection_allocation::ProjectionAllocation>(
    values: &mut String,
    value: &str,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<(), CasePreparationError<C, A::Error>> {
    check().map_err(CasePreparationError::Checkpoint)?;
    let needed = values.len().checked_add(value.len()).ok_or_else(|| {
        CasePreparationError::Allocation(A::Error::from(
            oryn_dom::DomError::SemanticProjectionCapacity,
        ))
    })?;
    oryn_dom::projection_allocation::reserve_string(values, needed, allocation)
        .map_err(CasePreparationError::Allocation)?;
    for ch in value.chars() {
        check().map_err(CasePreparationError::Checkpoint)?;
        values.push(ch);
    }
    Ok(())
}

impl CaseContext {
    fn build_with_allocation<C, A: oryn_dom::projection_allocation::ProjectionAllocation>(
        text: &str,
        greek_eligible: Option<&[bool]>,
        allocation: &mut A,
        checkpoint: &mut impl FnMut() -> Result<(), C>,
    ) -> Result<Self, CasePreparationError<C, A::Error>> {
        checkpoint().map_err(CasePreparationError::Checkpoint)?;
        let mut chars = Vec::new();
        for ch in text.chars() {
            checkpoint().map_err(CasePreparationError::Checkpoint)?;
            case_push(&mut chars, ch, allocation, checkpoint)?;
        }
        let count = chars.len();
        let mut context = Self {
            chars,
            greek_upper: std::collections::BTreeMap::new(),
            word_initial: case_flags(count, allocation, checkpoint)?,
            initial_grapheme: case_flags(count, allocation, checkpoint)?,
            before_cased: case_flags(count, allocation, checkpoint)?,
            after_cased: case_flags(count, allocation, checkpoint)?,
            after_i: case_flags(count, allocation, checkpoint)?,
            after_soft_dotted: case_flags(count, allocation, checkpoint)?,
            more_above: case_flags(count, allocation, checkpoint)?,
            before_dot: case_flags(count, allocation, checkpoint)?,
        };
        let cased = CodePointSetData::new::<Cased>();
        // Segment the complete paragraph so inline/style boundaries cannot
        // accidentally start a new capitalized word. Numbers consume the initial
        // typographic unit too; initial punctuation does not.
        let mut offset = 0;
        for word in text.split_word_bounds() {
            checkpoint().map_err(CasePreparationError::Checkpoint)?;
            let mut found = false;
            for ch in word.chars() {
                checkpoint().map_err(CasePreparationError::Checkpoint)?;
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
            checkpoint().map_err(CasePreparationError::Checkpoint)?;
            let mut end = first;
            for _ in grapheme.chars() {
                checkpoint().map_err(CasePreparationError::Checkpoint)?;
                end += 1;
            }
            let mut initial = false;
            for value in &context.word_initial[first..end] {
                checkpoint().map_err(CasePreparationError::Checkpoint)?;
                if *value {
                    initial = true;
                    break;
                }
            }
            if initial {
                for value in &mut context.initial_grapheme[first..end] {
                    checkpoint().map_err(CasePreparationError::Checkpoint)?;
                    *value = true;
                }
            }
            first = end;
        }
        let ignorable = CodePointSetData::new::<CaseIgnorable>();
        let soft_dotted = CodePointSetData::new::<SoftDotted>();
        let combining = CodePointMapData::<CanonicalCombiningClass>::new();
        let (mut preceding_cased, mut preceding_i, mut preceding_soft) = (false, false, false);
        for (index, &ch) in context.chars.iter().enumerate() {
            checkpoint().map_err(CasePreparationError::Checkpoint)?;
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
            checkpoint().map_err(CasePreparationError::Checkpoint)?;
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
        context.greek_upper = crate::greek_upper::replacements_with_allocation(
            &context.chars,
            &context.before_cased,
            &context.after_cased,
            greek_eligible,
            allocation,
            checkpoint,
        )?;
        Ok(context)
    }
}
fn case_flags<C, A: oryn_dom::projection_allocation::ProjectionAllocation>(
    count: usize,
    allocation: &mut A,
    checkpoint: &mut impl FnMut() -> Result<(), C>,
) -> Result<Vec<bool>, CasePreparationError<C, A::Error>> {
    checkpoint().map_err(CasePreparationError::Checkpoint)?;
    let mut values = Vec::new();
    oryn_dom::projection_allocation::reserve_vector(&mut values, count, allocation)
        .map_err(CasePreparationError::Allocation)?;
    for _ in 0..count {
        checkpoint().map_err(CasePreparationError::Checkpoint)?;
        values.push(false);
    }
    Ok(values)
}

fn case_text<C, A: oryn_dom::projection_allocation::ProjectionAllocation>(
    value: &str,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<String, CasePreparationError<C, A::Error>> {
    let mut output = String::new();
    case_append(&mut output, value, allocation, check)?;
    Ok(output)
}
fn case_scalar<C, A: oryn_dom::projection_allocation::ProjectionAllocation>(
    value: char,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<String, CasePreparationError<C, A::Error>> {
    let mut buffer = [0u8; 4];
    case_text(value.encode_utf8(&mut buffer), allocation, check)
}
fn case_append_scalar<C, A: oryn_dom::projection_allocation::ProjectionAllocation>(
    output: &mut String,
    value: char,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<(), CasePreparationError<C, A::Error>> {
    let mut buffer = [0u8; 4];
    case_append(output, value.encode_utf8(&mut buffer), allocation, check)
}

fn titlecase_with_allocation<C, A: oryn_dom::projection_allocation::ProjectionAllocation>(
    ch: char,
    allocation: &mut A,
    checkpoint: &mut impl FnMut() -> Result<(), C>,
) -> Result<String, CasePreparationError<C, A::Error>> {
    let scalar = match ch as u32 {
        0x01c4..=0x01c6 => Some(0x01c5),
        0x01c7..=0x01c9 => Some(0x01c8),
        0x01ca..=0x01cc => Some(0x01cb),
        0x01f1..=0x01f3 => Some(0x01f2),
        0x10d0..=0x10fa | 0x10fd..=0x10ff => return case_scalar(ch, allocation, checkpoint),
        0x1f80..=0x1f87 | 0x1f90..=0x1f97 | 0x1fa0..=0x1fa7 => Some(ch as u32 + 8),
        0x1f88..=0x1f8f | 0x1f98..=0x1f9f | 0x1fa8..=0x1faf => {
            return case_scalar(ch, allocation, checkpoint);
        }
        0x1fb3 | 0x1fbc => Some(0x1fbc),
        0x1fc3 | 0x1fcc => Some(0x1fcc),
        0x1ff3 | 0x1ffc => Some(0x1ffc),
        _ => None,
    };
    if let Some(scalar) = scalar {
        return case_scalar(char::from_u32(scalar).unwrap(), allocation, checkpoint);
    }
    match ch {
        '\u{1fb2}' => return case_text("\u{1fba}\u{345}", allocation, checkpoint),
        '\u{1fb4}' => return case_text("\u{386}\u{345}", allocation, checkpoint),
        '\u{1fb7}' => return case_text("Α\u{342}\u{345}", allocation, checkpoint),
        '\u{1fc2}' => return case_text("\u{1fca}\u{345}", allocation, checkpoint),
        '\u{1fc4}' => return case_text("\u{389}\u{345}", allocation, checkpoint),
        '\u{1fc7}' => return case_text("Η\u{342}\u{345}", allocation, checkpoint),
        '\u{1ff2}' => return case_text("\u{1ffa}\u{345}", allocation, checkpoint),
        '\u{1ff4}' => return case_text("\u{38f}\u{345}", allocation, checkpoint),
        '\u{1ff7}' => return case_text("Ω\u{342}\u{345}", allocation, checkpoint),
        _ => {}
    }
    let cased = CodePointSetData::new::<Cased>();
    let mut first_cased = false;
    let mut result = String::new();
    for upper in ch.to_uppercase() {
        if first_cased {
            for lower in upper.to_lowercase() {
                case_append_scalar(&mut result, lower, allocation, checkpoint)?;
            }
        } else {
            case_append_scalar(&mut result, upper, allocation, checkpoint)?;
            first_cased = cased.contains(upper);
        }
    }
    Ok(result)
}

impl CaseContext {
    fn convert_case_with_allocation<C, A: oryn_dom::projection_allocation::ProjectionAllocation>(
        &self,
        index: usize,
        transform: &str,
        language: &str,
        allocation: &mut A,
        checkpoint: &mut impl FnMut() -> Result<(), C>,
    ) -> Result<String, CasePreparationError<C, A::Error>> {
        let ch = self.chars[index];
        let turkic = case_language(language, "tr", checkpoint)
            .map_err(CasePreparationError::Checkpoint)?
            || case_language(language, "az", checkpoint)
                .map_err(CasePreparationError::Checkpoint)?;
        let lithuanian =
            case_language(language, "lt", checkpoint).map_err(CasePreparationError::Checkpoint)?;
        // Combined CSS values still select at most one case transformation.
        let uppercase = case_has_token(transform, "uppercase", checkpoint)
            .map_err(CasePreparationError::Checkpoint)?;
        let lowercase = case_has_token(transform, "lowercase", checkpoint)
            .map_err(CasePreparationError::Checkpoint)?;
        let capitalize = case_has_token(transform, "capitalize", checkpoint)
            .map_err(CasePreparationError::Checkpoint)?;
        if capitalize {
            // Titlecasing affects the initial typographic unit, including its
            // dependent marks, but never recases the remainder of the word.
            if lithuanian
                && ch == '\u{0307}'
                && self.initial_grapheme[index]
                && self.after_soft_dotted[index]
            {
                return Ok(String::new());
            }
            let dutch_ij = case_language(language, "nl", checkpoint)
                .map_err(CasePreparationError::Checkpoint)?
                && index > 0
                && matches!(ch, 'j' | 'J')
                && matches!(self.chars[index - 1], 'i' | 'I')
                && self.word_initial[index - 1];
            if self.word_initial[index] || dutch_ij {
                if turkic && ch == 'i' {
                    return case_text("İ", allocation, checkpoint);
                }
                return titlecase_with_allocation(ch, allocation, checkpoint);
            }
            return case_scalar(ch, allocation, checkpoint);
        }
        if uppercase {
            if case_language(language, "el", checkpoint)
                .map_err(CasePreparationError::Checkpoint)?
                && let Some(rendered) = self.greek_upper.get(&index)
            {
                return case_text(rendered, allocation, checkpoint);
            }
            if turkic && ch == 'i' {
                return case_text("İ", allocation, checkpoint);
            }
            if lithuanian && ch == '\u{0307}' && self.after_soft_dotted[index] {
                return Ok(String::new());
            }
            let mut result = String::new();
            for upper in ch.to_uppercase() {
                case_append_scalar(&mut result, upper, allocation, checkpoint)?;
            }
            return Ok(result);
        }
        if lowercase {
            if ch == 'Σ' && self.before_cased[index] && !self.after_cased[index] {
                return case_text("ς", allocation, checkpoint);
            }
            if turkic {
                match ch {
                    'İ' => return case_text("i", allocation, checkpoint),
                    'I' if !self.before_dot[index] => {
                        return case_text("ı", allocation, checkpoint);
                    }
                    '\u{0307}' if self.after_i[index] => return Ok(String::new()),
                    _ => {}
                }
            }
            if lithuanian {
                match ch {
                    'I' | 'J' | 'Į' if self.more_above[index] => {
                        let mut result = String::new();
                        for lower in ch.to_lowercase() {
                            case_append_scalar(&mut result, lower, allocation, checkpoint)?;
                        }
                        case_append_scalar(&mut result, '\u{0307}', allocation, checkpoint)?;
                        return Ok(result);
                    }
                    'Ì' => return case_text("i\u{0307}\u{0300}", allocation, checkpoint),
                    'Í' => return case_text("i\u{0307}\u{0301}", allocation, checkpoint),
                    'Ĩ' => return case_text("i\u{0307}\u{0303}", allocation, checkpoint),
                    _ => {}
                }
            }
            let mut result = String::new();
            for lower in ch.to_lowercase() {
                case_append_scalar(&mut result, lower, allocation, checkpoint)?;
            }
            return Ok(result);
        }
        case_scalar(ch, allocation, checkpoint)
    }
}

fn case_has_token<C>(
    value: &str,
    wanted: &str,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<bool, C> {
    let mut start = None;
    for (index, ch) in value.char_indices() {
        check()?;
        if ch.is_ascii_whitespace() {
            if let Some(first) = start.take()
                && &value[first..index] == wanted
            {
                return Ok(true);
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    check()?;
    Ok(start.is_some_and(|first| &value[first..] == wanted))
}
fn case_language<C>(
    value: &str,
    wanted: &str,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<bool, C> {
    check()?;
    if value.len() != wanted.len() {
        return Ok(false);
    }
    for (left, right) in value.bytes().zip(wanted.bytes()) {
        check()?;
        if !left.eq_ignore_ascii_case(&right) {
            return Ok(false);
        }
    }
    Ok(true)
}

impl CaseContext {
    fn convert_with_allocation<C, A: oryn_dom::projection_allocation::ProjectionAllocation>(
        &self,
        index: usize,
        transform: &str,
        language: &str,
        allocation: &mut A,
        check: &mut impl FnMut() -> Result<(), C>,
    ) -> Result<String, CasePreparationError<C, A::Error>> {
        let width = case_has_token(transform, "full-width", check)
            .map_err(CasePreparationError::Checkpoint)?;
        let kana = case_has_token(transform, "full-size-kana", check)
            .map_err(CasePreparationError::Checkpoint)?;
        let converted =
            self.convert_case_with_allocation(index, transform, language, allocation, check)?;
        let mut output = String::new();
        for ch in converted.chars() {
            check().map_err(CasePreparationError::Checkpoint)?;
            let ch = if width { full_width(ch) } else { ch };
            let ch = if kana { full_size_kana(ch) } else { ch };
            case_append_scalar(&mut output, ch, allocation, check)?;
        }
        Ok(output)
    }
}

pub fn accessibility_case_fragment_with_allocation<
    C,
    A: oryn_dom::projection_allocation::ProjectionAllocation,
>(
    text: &str,
    start: usize,
    end: usize,
    transform: &str,
    language: &str,
    allocation: &mut A,
    check: &mut impl FnMut() -> Result<(), C>,
) -> Result<String, CasePreparationError<C, A::Error>> {
    check().map_err(CasePreparationError::Checkpoint)?;
    let mut case = "none";
    let mut first = None;
    for (index, ch) in transform
        .char_indices()
        .chain(std::iter::once((transform.len(), ' ')))
    {
        check().map_err(CasePreparationError::Checkpoint)?;
        if ch.is_ascii_whitespace() {
            if let Some(start) = first.take() {
                let token = &transform[start..index];
                if matches!(token, "uppercase" | "lowercase" | "capitalize") {
                    case = token;
                    break;
                }
            }
        } else if first.is_none() {
            first = Some(index);
        }
    }
    let context_start = allocation.scratch_mark();
    let mut count = 0usize;
    let mut eligible = Vec::new();
    for _ in text.chars() {
        check().map_err(CasePreparationError::Checkpoint)?;
        count = count.checked_add(1).ok_or_else(|| {
            CasePreparationError::Allocation(A::Error::from(
                oryn_dom::DomError::SemanticProjectionCapacity,
            ))
        })?;
        case_push(&mut eligible, case == "uppercase", allocation, check)?;
    }
    let start = start.min(count);
    let end = end.min(count).max(start);
    let context = CaseContext::build_with_allocation(text, Some(&eligible), allocation, check)?;
    let context_end = allocation.scratch_mark();
    let mut output = String::new();
    for index in start..end {
        check().map_err(CasePreparationError::Checkpoint)?;
        let mark = allocation.scratch_mark();
        let value = context.convert_with_allocation(index, case, language, allocation, check)?;
        let end = allocation.scratch_mark();
        case_append(&mut output, &value, allocation, check)?;
        drop(value);
        allocation.release_scratch_range(mark..end);
    }
    drop(context);
    drop(eligible);
    allocation.release_scratch_range(context_start..context_end);
    Ok(output)
}

pub(super) fn ordinary_case_result<T, C>(
    result: Result<T, CasePreparationError<C, oryn_dom::DomError>>,
) -> Result<T, C> {
    match result {
        Ok(value) => Ok(value),
        Err(CasePreparationError::Checkpoint(error)) => Err(error),
        Err(CasePreparationError::Allocation(error)) => {
            panic!("ordinary case capacity failure: {error}")
        }
    }
}
impl CaseContext {
    fn build_with_checkpoint<E>(
        text: &str,
        eligible: Option<&[bool]>,
        checkpoint: &mut impl FnMut() -> Result<(), E>,
    ) -> Result<Self, E> {
        ordinary_case_result(Self::build_with_allocation(
            text,
            eligible,
            &mut oryn_dom::projection_allocation::OrdinaryProjectionAllocation,
            checkpoint,
        ))
    }
    pub fn convert(&self, index: usize, transform: &str, language: &str) -> String {
        match ordinary_case_result(self.convert_with_allocation(
            index,
            transform,
            language,
            &mut oryn_dom::projection_allocation::OrdinaryProjectionAllocation,
            &mut || Ok::<_, std::convert::Infallible>(()),
        )) {
            Ok(value) => value,
            Err(never) => match never {},
        }
    }
}
pub fn accessibility_case_fragment_with_checkpoint<E>(
    text: &str,
    start: usize,
    end: usize,
    transform: &str,
    language: &str,
    checkpoint: &mut impl FnMut() -> Result<(), E>,
) -> Result<String, E> {
    ordinary_case_result(accessibility_case_fragment_with_allocation(
        text,
        start,
        end,
        transform,
        language,
        &mut oryn_dom::projection_allocation::OrdinaryProjectionAllocation,
        checkpoint,
    ))
}
#[cfg(test)]
fn titlecase(ch: char) -> String {
    match ordinary_case_result(titlecase_with_allocation(
        ch,
        &mut oryn_dom::projection_allocation::OrdinaryProjectionAllocation,
        &mut || Ok::<_, std::convert::Infallible>(()),
    )) {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

#[cfg(test)]
mod case_allocation_controls {
    use super::*;
    #[derive(Debug)]
    struct Error;
    impl From<oryn_dom::DomError> for Error {
        fn from(_: oryn_dom::DomError) -> Self {
            Self
        }
    }
    struct Allocation {
        calls: usize,
        fail: usize,
    }
    impl oryn_dom::projection_allocation::ProjectionAllocation for Allocation {
        type Error = Error;
        fn reserve(&mut self, _: usize) -> Result<(), Error> {
            self.calls += 1;
            if self.calls == self.fail {
                Err(Error)
            } else {
                Ok(())
            }
        }
        fn allocation_error(&self, _: std::collections::TryReserveError) -> Error {
            Error
        }
    }
    #[test]
    fn multilingual_allocated_case_matches_ordinary_and_preserves_source() {
        for (text, transform, language) in [
            ("hello world", "capitalize", "en"),
            ("ΟΣ ΟΣΑ", "lowercase", "el"),
            ("ή άι α\u{301}ι", "uppercase", "el"),
            ("I\u{307} İ", "lowercase", "tr"),
            ("I\u{301}", "lowercase", "lt"),
        ] {
            let mut allocation = Allocation {
                calls: 0,
                fail: usize::MAX,
            };
            let value = accessibility_case_fragment_with_allocation(
                text,
                0,
                text.chars().count(),
                transform,
                language,
                &mut allocation,
                &mut || Ok::<_, oryn_dom::DomError>(()),
            )
            .unwrap();
            assert_eq!(
                value,
                accessibility_case_fragment(text, 0, text.chars().count(), transform, language)
            );
            let count = allocation.calls;
            for fail in 1..=count {
                let mut allocation = Allocation { calls: 0, fail };
                assert!(
                    accessibility_case_fragment_with_allocation(
                        text,
                        0,
                        text.chars().count(),
                        transform,
                        language,
                        &mut allocation,
                        &mut || Ok::<_, oryn_dom::DomError>(())
                    )
                    .is_err()
                );
            }
        }
    }
    #[test]
    fn long_fragment_cancellation_preserves_entry_loop() {
        let text = "a\u{301}".repeat(8192);
        let mut allocation = Allocation {
            calls: 0,
            fail: usize::MAX,
        };
        let mut calls = 0;
        let result = accessibility_case_fragment_with_allocation(
            &text,
            0,
            16384,
            "uppercase",
            "el",
            &mut allocation,
            &mut || {
                calls += 1;
                if calls == 64 {
                    Err(oryn_dom::DomError::ProjectionCancelled)
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(
            result,
            Err(CasePreparationError::Checkpoint(
                oryn_dom::DomError::ProjectionCancelled
            ))
        ));
        assert_eq!(calls, 64);
    }
    #[test]
    fn greek_combining_loop_cancels_without_fragment_prescan() {
        let chars: Vec<char> = format!("α{}", "\u{301}".repeat(8192)).chars().collect();
        let before = vec![false; chars.len()];
        let after = vec![false; chars.len()];
        let mut allocation = Allocation { calls: 0, fail: usize::MAX };
        let mut calls = 0;
        let result = crate::greek_upper::replacements_with_allocation(
            &chars, &before, &after, None, &mut allocation, &mut || {
                calls += 1;
                if calls == 64 { Err(oryn_dom::DomError::ProjectionCancelled) } else { Ok(()) }
            });
        assert!(matches!(result, Err(CasePreparationError::Checkpoint(
            oryn_dom::DomError::ProjectionCancelled))));
        assert_eq!(calls, 64);
        assert!(allocation.calls > 0, "decomposition was reached");
    }

    #[test]
    fn flag_allocation_observes_entry_cancellation_even_when_empty() {
        let mut allocation = Allocation { calls: 0, fail: usize::MAX };
        let result = case_flags(0, &mut allocation, &mut ||
            Err(oryn_dom::DomError::ProjectionCancelled));
        assert!(matches!(result, Err(CasePreparationError::Checkpoint(
            oryn_dom::DomError::ProjectionCancelled))));
        assert_eq!(allocation.calls, 0);
    }

}
