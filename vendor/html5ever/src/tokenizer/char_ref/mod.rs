// Copyright 2014-2017 The html5ever Project Developers. See the
// COPYRIGHT file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use super::{TokenSink, Tokenizer};
use crate::buffer_queue::BufferQueue;
use crate::data;
use crate::tendril::StrTendril;

use log::debug;
use std::borrow::Cow::{self, Borrowed};
use std::char::from_u32;

trait ReferenceContext {
    fn peek(&self) -> Option<char>;
    fn discard(&mut self);
    fn start_name(&mut self, storage: &mut Option<StrTendril>);
    fn append_name(&mut self, storage: &mut Option<StrTendril>, ch: char);
    fn name<'a>(&'a self, storage: &'a Option<StrTendril>) -> &'a str;
    fn replay_name(&mut self, storage: &mut Option<StrTendril>, offset: usize);
    fn replay_numeric(&mut self, hex: Option<char>);
    fn exact_errors(&self) -> bool;
    fn emit_error(&mut self, error: Cow<'static, str>);
    fn reset_ignore_lf(&mut self);
}
struct TokenizerContext<'a, S: TokenSink> {
    tokenizer: &'a Tokenizer<S>,
    input: &'a BufferQueue,
}
impl<S: TokenSink> ReferenceContext for TokenizerContext<'_, S> {
    fn peek(&self) -> Option<char> {
        self.tokenizer.peek(self.input)
    }
    fn discard(&mut self) {
        self.tokenizer.discard_char(self.input);
    }
    fn start_name(&mut self, storage: &mut Option<StrTendril>) {
        *storage = Some(StrTendril::new());
    }
    fn append_name(&mut self, storage: &mut Option<StrTendril>, ch: char) {
        storage.as_mut().unwrap().push_char(ch);
    }
    fn name<'a>(&'a self, storage: &'a Option<StrTendril>) -> &'a str {
        storage.as_ref().unwrap()
    }
    fn replay_name(&mut self, storage: &mut Option<StrTendril>, offset: usize) {
        if offset == 0 {
            self.input.push_front(storage.take().unwrap());
        } else {
            self.input
                .push_front(StrTendril::from_slice(&storage.as_ref().unwrap()[offset..]));
        }
    }
    fn replay_numeric(&mut self, hex: Option<char>) {
        let mut value = StrTendril::from_char('#');
        if let Some(ch) = hex {
            value.push_char(ch);
        }
        self.input.push_front(value);
    }
    fn exact_errors(&self) -> bool {
        self.tokenizer.opts.exact_errors
    }
    fn emit_error(&mut self, error: Cow<'static, str>) {
        self.tokenizer.emit_error(error);
    }
    fn reset_ignore_lf(&mut self) {
        self.tokenizer.ignore_lf.set(false);
    }
}
struct BorrowedContext<'a> {
    input: &'a str,
    cursor: usize,
    name_start: usize,
}
impl ReferenceContext for BorrowedContext<'_> {
    fn peek(&self) -> Option<char> {
        self.input[self.cursor..].chars().next()
    }
    fn discard(&mut self) {
        self.cursor += self.peek().unwrap().len_utf8();
    }
    fn start_name(&mut self, _: &mut Option<StrTendril>) {
        self.name_start = self.cursor;
    }
    fn append_name(&mut self, _: &mut Option<StrTendril>, _: char) {}
    fn name<'a>(&'a self, _: &'a Option<StrTendril>) -> &'a str {
        &self.input[self.name_start..self.cursor]
    }
    fn replay_name(&mut self, _: &mut Option<StrTendril>, offset: usize) {
        self.cursor = self.name_start + offset;
    }
    fn replay_numeric(&mut self, hex: Option<char>) {
        self.cursor -= 1 + usize::from(hex.is_some());
    }
    fn exact_errors(&self) -> bool {
        false
    }
    fn emit_error(&mut self, _: Cow<'static, str>) {}
    fn reset_ignore_lf(&mut self) {}
}

impl CharRefTokenizer {
    pub(super) fn step<S: TokenSink>(
        &mut self,
        tokenizer: &Tokenizer<S>,
        input: &BufferQueue,
    ) -> Status {
        self.step_context(&mut TokenizerContext { tokenizer, input })
    }
    pub(super) fn end_of_file<S: TokenSink>(
        &mut self,
        tokenizer: &Tokenizer<S>,
        input: &BufferQueue,
    ) -> CharRef {
        self.end_of_file_context(&mut TokenizerContext { tokenizer, input })
    }
}

/// Decode references in an attribute value using the canonical tokenizer grammar.
/// Input and name lookahead are borrowed; output growth is admitted before allocation.
pub fn decode_attribute_references<E>(
    value: &str,
    reserve: &mut impl FnMut(usize) -> Result<(), E>,
    check: &mut impl FnMut() -> Result<(), E>,
    allocation_error: &mut impl FnMut(std::collections::TryReserveError) -> E,
) -> Result<String, E> {
    fn append<E>(
        output: &mut String,
        ch: char,
        reserve: &mut impl FnMut(usize) -> Result<(), E>,
        check: &mut impl FnMut() -> Result<(), E>,
        allocation_error: &mut impl FnMut(std::collections::TryReserveError) -> E,
    ) -> Result<(), E> {
        check()?;
        let needed = output
            .len()
            .checked_add(ch.len_utf8())
            .unwrap_or(usize::MAX);
        if needed > output.capacity() {
            if needed > isize::MAX as usize {
                let error = Vec::<u8>::new().try_reserve(usize::MAX).unwrap_err();
                return Err(allocation_error(error));
            }
            let capacity = output
                .capacity()
                .saturating_mul(2)
                .max(needed)
                .max(8)
                .min(isize::MAX as usize);
            reserve(capacity.saturating_sub(output.capacity()))?;
            output
                .try_reserve_exact(capacity.saturating_sub(output.len()))
                .map_err(allocation_error)?;
            // Allocator rounding may exceed the requested capacity. This
            // additional admission occurs after allocation but before copying
            // or publishing; refusal drops the private output immediately.
            if output.capacity() > capacity {
                reserve(output.capacity() - capacity)?;
            }
        }
        output.push(ch);
        Ok(())
    }
    check()?;
    let mut context = BorrowedContext {
        input: value,
        cursor: 0,
        name_start: 0,
    };
    let mut output = String::new();
    while let Some(ch) = context.peek() {
        check()?;
        context.discard();
        if ch != '&' {
            append(&mut output, ch, reserve, check, allocation_error)?;
            continue;
        }
        let mut tokenizer = CharRefTokenizer::new(true);
        let reference = loop {
            check()?;
            match tokenizer.step_context(&mut context) {
                Status::Progress => {}
                Status::Done(reference) => break reference,
                Status::Stuck => break tokenizer.end_of_file_context(&mut context),
            }
        };
        if reference.num_chars == 0 {
            append(&mut output, '&', reserve, check, allocation_error)?;
        } else {
            for ch in &reference.chars[..usize::from(reference.num_chars)] {
                append(&mut output, *ch, reserve, check, allocation_error)?;
            }
        }
    }
    check()?;
    Ok(output)
}

#[cfg(test)]
mod borrowed_reference_tests {
    use super::*;
    struct IgnoreErrors;
    impl TokenSink for IgnoreErrors {
        type Handle = ();
        fn process_token(
            &self,
            _: super::super::Token,
            _: u64,
        ) -> super::super::TokenSinkResult<()> {
            super::super::TokenSinkResult::Continue
        }
    }
    fn ordinary_reference_decode(value: &str) -> String {
        let tokenizer = Tokenizer::new(IgnoreErrors, super::super::TokenizerOpts::default());
        let input = BufferQueue::default();
        input.push_back(StrTendril::from_slice(value));
        let mut output = String::new();
        while let Some(ch) = input.next() {
            if ch != '&' {
                output.push(ch);
                continue;
            }
            let mut reference = CharRefTokenizer::new(true);
            let decoded = loop {
                match reference.step(&tokenizer, &input) {
                    Status::Progress => {}
                    Status::Done(value) => break value,
                    Status::Stuck => break reference.end_of_file(&tokenizer, &input),
                }
            };
            if decoded.num_chars == 0 {
                output.push('&');
            } else {
                output.extend(&decoded.chars[..usize::from(decoded.num_chars)]);
            }
        }
        output
    }
    #[test]
    fn every_canonical_entity_and_prefix_matches_ordinary_tokenizer_adapter() {
        for (name, _) in data::NAMED_ENTITIES.entries() {
            for prefix in 0..=name.len() {
                for suffix in ["", "=", "x", "é", "\r\n", "\0"] {
                    let input = format!("&{}{suffix}", &name[..prefix]);
                    let actual = decode_attribute_references(
                        &input,
                        &mut |_| Ok::<_, ()>(()),
                        &mut || Ok(()),
                        &mut |_| (),
                    )
                    .unwrap();
                    assert_eq!(actual, ordinary_reference_decode(&input), "{input:?}");
                }
            }
        }
        for input in [
            "&",
            "&#",
            "&#x",
            "&#X",
            "&#x;",
            "&#42",
            "&#x80;",
            "&#0;",
            "&#xD800;",
            "&#1114112;",
            "&#999999999999999999999999999999;",
            "α\r\n\0&NotEqualTilde;β",
        ] {
            let actual = decode_attribute_references(
                input,
                &mut |_| Ok::<_, ()>(()),
                &mut || Ok(()),
                &mut |_| (),
            )
            .unwrap();
            assert_eq!(actual, ordinary_reference_decode(input), "{input:?}");
        }
    }
    #[test]
    fn canonical_attribute_cases_and_eof() {
        for (input, expected) in [
            ("", ""),
            ("&amp;", "&"),
            ("&amp=", "&amp="),
            ("&ampx", "&ampx"),
            ("&notit;", "&notit;"),
            ("&not;", "¬"),
            ("&NotEqualTilde;", "≂̸"),
            ("&#", "&#"),
            ("&#x", "&#x"),
            ("&#Xz", "&#Xz"),
            ("&#65", "A"),
            ("&#x80;", "€"),
            ("&#0;", "�"),
            ("&#xD800;", "�"),
            ("&#1114112;", "�"),
            ("&#999999999999999999999999999999;", "�"),
            ("&unknown;", "&unknown;"),
        ] {
            let actual = decode_attribute_references(
                input,
                &mut |_| Ok::<_, ()>(()),
                &mut || Ok(()),
                &mut |_| (),
            )
            .unwrap();
            assert_eq!(actual, expected, "{input:?}");
        }
    }
    #[test]
    fn actual_long_bogus_scan_and_all_reservation_failures() {
        let input = format!("&{};", "z".repeat(65_536));
        let mut calls = 0;
        assert!(decode_attribute_references(
            &input,
            &mut |_| Ok::<_, ()>(()),
            &mut || {
                calls += 1;
                if calls == 4096 {
                    Err(())
                } else {
                    Ok(())
                }
            },
            &mut |_| ()
        )
        .is_err());
        let mut reservations = 0;
        let result = decode_attribute_references(
            &input,
            &mut |_| {
                reservations += 1;
                Ok::<_, ()>(())
            },
            &mut || Ok(()),
            &mut |_| (),
        )
        .unwrap();
        assert_eq!(result, input);
        for failure in 1..=reservations {
            let mut count = 0;
            assert!(decode_attribute_references(
                &input,
                &mut |_| {
                    count += 1;
                    if count == failure {
                        Err(())
                    } else {
                        Ok(())
                    }
                },
                &mut || Ok(()),
                &mut |_| ()
            )
            .is_err());
        }
        assert!(decode_attribute_references(
            "",
            &mut |_| Ok::<_, ()>(()),
            &mut || Err(()),
            &mut |_| ()
        )
        .is_err());
    }
}

//§ tokenizing-character-references
pub(super) struct CharRef {
    /// The resulting character(s)
    pub(super) chars: [char; 2],

    /// How many slots in `chars` are valid?
    pub(super) num_chars: u8,
}

pub(super) enum Status {
    Stuck,
    Progress,
    Done(CharRef),
}

#[derive(Debug)]
enum State {
    Begin,
    Octothorpe,
    Numeric(u32), // base
    NumericSemicolon,
    Named,
    BogusName,
}

pub(super) struct CharRefTokenizer {
    state: State,
    is_consumed_in_attribute: bool,

    num: u32,
    num_too_big: bool,
    seen_digit: bool,
    hex_marker: Option<char>,

    name_buf_opt: Option<StrTendril>,
    name_match: Option<(u32, u32)>,
    name_len: usize,
}

impl CharRef {
    const EMPTY: CharRef = CharRef {
        chars: ['\0', '\0'],
        num_chars: 0,
    };
}

impl CharRefTokenizer {
    pub(super) fn retained_bytes(&self) -> usize {
        self.name_buf_opt.as_ref().map_or(0, |name| name.len())
    }

    pub(super) fn new(is_consumed_in_attribute: bool) -> CharRefTokenizer {
        CharRefTokenizer {
            is_consumed_in_attribute,
            state: State::Begin,
            num: 0,
            num_too_big: false,
            seen_digit: false,
            hex_marker: None,
            name_buf_opt: None,
            name_match: None,
            name_len: 0,
        }
    }

    fn finish_one(&mut self, c: char) -> Status {
        Status::Done(CharRef {
            chars: [c, '\0'],
            num_chars: 1,
        })
    }
}

impl CharRefTokenizer {
    fn step_context<C: ReferenceContext>(&mut self, context: &mut C) -> Status {
        debug!("char ref tokenizer stepping in state {:?}", self.state);
        match self.state {
            State::Begin => self.do_begin(context),
            State::Octothorpe => self.do_octothorpe(context),
            State::Numeric(base) => self.do_numeric(context, base),
            State::NumericSemicolon => self.do_numeric_semicolon(context),
            State::Named => self.do_named(context),
            State::BogusName => self.do_bogus_name(context),
        }
    }

    fn do_begin<C: ReferenceContext>(&mut self, context: &mut C) -> Status {
        match context.peek() {
            Some('a'..='z' | 'A'..='Z' | '0'..='9') => {
                self.state = State::Named;
                context.start_name(&mut self.name_buf_opt);
                Status::Progress
            }
            Some('#') => {
                context.discard();
                self.state = State::Octothorpe;
                Status::Progress
            }
            Some(_) => Status::Done(CharRef::EMPTY),
            None => Status::Stuck,
        }
    }

    fn do_octothorpe<C: ReferenceContext>(&mut self, context: &mut C) -> Status {
        match context.peek() {
            Some(c @ ('x' | 'X')) => {
                context.discard();
                self.hex_marker = Some(c);
                self.state = State::Numeric(16);
            }
            Some(_) => {
                self.hex_marker = None;
                self.state = State::Numeric(10);
            }
            None => return Status::Stuck,
        }
        Status::Progress
    }

    fn do_numeric<C: ReferenceContext>(&mut self, context: &mut C, base: u32) -> Status {
        let Some(c) = context.peek() else {
            return Status::Stuck;
        };
        match c.to_digit(base) {
            Some(n) => {
                context.discard();
                self.num = self.num.wrapping_mul(base);
                if self.num > 0x10FFFF {
                    // We might overflow, and the character is definitely invalid.
                    // We still parse digits and semicolon, but don't use the result.
                    self.num_too_big = true;
                }
                self.num = self.num.wrapping_add(n);
                self.seen_digit = true;
                Status::Progress
            }

            None if !self.seen_digit => self.unconsume_numeric(context),

            None => {
                self.state = State::NumericSemicolon;
                Status::Progress
            }
        }
    }

    fn do_numeric_semicolon<C: ReferenceContext>(&mut self, context: &mut C) -> Status {
        match context.peek() {
            Some(';') => context.discard(),
            Some(_) => context.emit_error(Borrowed(
                "Semicolon missing after numeric character reference",
            )),
            None => return Status::Stuck,
        };
        self.finish_numeric(context)
    }

    fn unconsume_numeric<C: ReferenceContext>(&mut self, context: &mut C) -> Status {
        context.replay_numeric(self.hex_marker);
        context.emit_error(Borrowed("Numeric character reference without digits"));
        Status::Done(CharRef::EMPTY)
    }

    fn finish_numeric<C: ReferenceContext>(&mut self, context: &mut C) -> Status {
        fn conv(n: u32) -> char {
            from_u32(n).expect("invalid char missed by error handling cases")
        }

        let (c, error) = match self.num {
            n if (n > 0x10FFFF) || self.num_too_big => ('\u{fffd}', true),
            0x00 | 0xD800..=0xDFFF => ('\u{fffd}', true),

            0x80..=0x9F => match data::C1_REPLACEMENTS[(self.num - 0x80) as usize] {
                Some(c) => (c, true),
                None => (conv(self.num), true),
            },

            0x01..=0x08 | 0x0B | 0x0D..=0x1F | 0x7F | 0xFDD0..=0xFDEF => (conv(self.num), true),

            n if (n & 0xFFFE) == 0xFFFE => (conv(n), true),

            n => (conv(n), false),
        };

        if error {
            let msg = if context.exact_errors() {
                Cow::from(format!(
                    "Invalid numeric character reference value 0x{:06X}",
                    self.num
                ))
            } else {
                Cow::from("Invalid numeric character reference")
            };
            context.emit_error(msg);
        }

        self.finish_one(c)
    }

    fn do_named<C: ReferenceContext>(&mut self, context: &mut C) -> Status {
        // peek + discard skips over newline normalization, therefore making it easier to
        // un-consume
        let Some(c) = context.peek() else {
            return Status::Stuck;
        };
        context.discard();
        context.append_name(&mut self.name_buf_opt, c);
        match data::NAMED_ENTITIES.get(&context.name(&self.name_buf_opt)[..]) {
            // We have either a full match or a prefix of one.
            Some(&m) => {
                if m.0 != 0 {
                    // We have a full match, but there might be a longer one to come.
                    self.name_match = Some(m);
                    self.name_len = context.name(&self.name_buf_opt).len();
                }
                // Otherwise we just have a prefix match.
                Status::Progress
            }

            // Can't continue the match.
            None => self.finish_named(context, Some(c)),
        }
    }

    fn emit_name_error<C: ReferenceContext>(&mut self, context: &mut C) {
        let msg = if context.exact_errors() {
            Cow::from(format!(
                "Invalid character reference &{}",
                context.name(&self.name_buf_opt)
            ))
        } else {
            Cow::from("Invalid character reference")
        };
        context.emit_error(msg);
    }

    fn unconsume_name<C: ReferenceContext>(&mut self, context: &mut C) {
        context.replay_name(&mut self.name_buf_opt, 0);
    }

    fn finish_named<C: ReferenceContext>(
        &mut self,
        context: &mut C,
        end_char: Option<char>,
    ) -> Status {
        match self.name_match {
            None => {
                match end_char {
                    Some(c) if c.is_ascii_alphanumeric() => {
                        // Keep looking for a semicolon, to determine whether
                        // we emit a parse error.
                        self.state = State::BogusName;
                        return Status::Progress;
                    }

                    // Check length because &; is not a parse error.
                    Some(';') if context.name(&self.name_buf_opt).len() > 1 => {
                        self.emit_name_error(context)
                    }

                    _ => (),
                }
                self.unconsume_name(context);
                Status::Done(CharRef::EMPTY)
            }

            Some((c1, c2)) => {
                // We have a complete match, but we may have consumed
                // additional characters into self.name_buf.  Usually
                // at least one, but several in cases like
                //
                //     &not    => match for U+00AC
                //     &noti   => valid prefix for &notin
                //     &notit  => can't continue match

                let name_len = self.name_len;
                assert!(name_len > 0);
                let last_matched = context.name(&self.name_buf_opt)[name_len - 1..]
                    .chars()
                    .next()
                    .unwrap();

                // There might not be a next character after the match, if
                // we had a full match and then hit EOF.
                let next_after = if name_len == context.name(&self.name_buf_opt).len() {
                    None
                } else {
                    Some(
                        context.name(&self.name_buf_opt)[name_len..]
                            .chars()
                            .next()
                            .unwrap(),
                    )
                };

                // If the character reference was consumed as part of an attribute, and the last
                // character matched is not a U+003B SEMICOLON character (;), and the next input
                // character is either a U+003D EQUALS SIGN character (=) or an ASCII alphanumeric,
                // then, for historical reasons, flush code points consumed as a character
                // reference and switch to the return state.

                let unconsume_all = match (self.is_consumed_in_attribute, last_matched, next_after)
                {
                    (_, ';', _) => false,
                    (true, _, Some('=')) => true,
                    (true, _, Some(c)) if c.is_ascii_alphanumeric() => true,
                    _ => {
                        // 1. If the last character matched is not a U+003B SEMICOLON character
                        //    (;), then this is a missing-semicolon-after-character-reference parse
                        //    error.
                        context.emit_error(Borrowed(
                            "Character reference does not end with semicolon",
                        ));
                        false
                    }
                };

                if unconsume_all {
                    self.unconsume_name(context);
                    Status::Done(CharRef::EMPTY)
                } else {
                    context.replay_name(&mut self.name_buf_opt, name_len);
                    context.reset_ignore_lf();
                    Status::Done(CharRef {
                        chars: [from_u32(c1).unwrap(), from_u32(c2).unwrap()],
                        num_chars: if c2 == 0 { 1 } else { 2 },
                    })
                }
            }
        }
    }

    fn do_bogus_name<C: ReferenceContext>(&mut self, context: &mut C) -> Status {
        // peek + discard skips over newline normalization, therefore making it easier to
        // un-consume
        let Some(c) = context.peek() else {
            return Status::Stuck;
        };
        context.discard();
        context.append_name(&mut self.name_buf_opt, c);
        match c {
            _ if c.is_ascii_alphanumeric() => return Status::Progress,
            ';' => self.emit_name_error(context),
            _ => (),
        }
        self.unconsume_name(context);
        Status::Done(CharRef::EMPTY)
    }

    fn end_of_file_context<C: ReferenceContext>(&mut self, context: &mut C) -> CharRef {
        loop {
            let status = match self.state {
                State::Begin => Status::Done(CharRef::EMPTY),
                State::Numeric(_) if !self.seen_digit => self.unconsume_numeric(context),
                State::Numeric(_) | State::NumericSemicolon => {
                    context.emit_error(Borrowed("EOF in numeric character reference"));
                    self.finish_numeric(context)
                }
                State::Named => self.finish_named(context, None),
                State::BogusName => {
                    self.unconsume_name(context);
                    Status::Done(CharRef::EMPTY)
                }
                State::Octothorpe => {
                    context.replay_numeric(None);
                    context.emit_error(Borrowed("EOF after '#' in character reference"));
                    Status::Done(CharRef::EMPTY)
                }
            };

            match status {
                Status::Done(char_ref) => {
                    return char_ref;
                }
                Status::Stuck => {
                    return CharRef::EMPTY;
                }
                Status::Progress => {}
            }
        }
    }
}
