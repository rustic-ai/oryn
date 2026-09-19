//! Native Gregorian locale symbols. This reads platform ICU data through CF;
//! the returned bounded data, not an author formatter, drives control editing.
#[cfg(target_os = "macos")]
pub(super) fn profile(locale: &str) -> Option<oryn_dom::ControlLocale> {
    use std::ffi::{c_char, c_void};
    type CF = *const c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: CF);
        fn CFStringCreateWithBytes(
            allocator: CF,
            bytes: *const u8,
            length: isize,
            encoding: u32,
            external: u8,
        ) -> CF;
        fn CFStringGetCString(value: CF, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFLocaleCreate(allocator: CF, identifier: CF) -> CF;
        fn CFDateFormatterCreate(
            allocator: CF,
            locale: CF,
            date_style: isize,
            time_style: isize,
        ) -> CF;
        fn CFDateFormatterCopyProperty(formatter: CF, key: CF) -> CF;
        fn CFDateFormatterCreateDateFormatFromTemplate(
            allocator: CF,
            template: CF,
            options: u64,
            locale: CF,
        ) -> CF;
        fn CFArrayGetCount(array: CF) -> isize;
        fn CFArrayGetValueAtIndex(array: CF, index: isize) -> CF;
        fn CFCalendarCreateWithIdentifier(allocator: CF, identifier: CF) -> CF;
        fn CFCalendarSetLocale(calendar: CF, locale: CF);
        fn CFCalendarGetFirstWeekday(calendar: CF) -> isize;
        fn CFNumberFormatterCreate(allocator: CF, locale: CF, style: isize) -> CF;
        fn CFNumberFormatterCreateStringWithValue(
            allocator: CF,
            formatter: CF,
            kind: isize,
            value: CF,
        ) -> CF;
        static kCFDateFormatterMonthSymbols: CF;
        static kCFDateFormatterShortWeekdaySymbols: CF;
        static kCFGregorianCalendar: CF;
    }
    struct Owned(CF);
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) }
        }
    }
    fn owned(value: CF) -> Option<Owned> {
        if value.is_null() {
            None
        } else {
            Some(Owned(value))
        }
    }
    fn string(value: &str) -> Option<Owned> {
        owned(unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                value.as_ptr(),
                value.len() as isize,
                0x08000100,
                0,
            )
        })
    }
    fn text(value: CF) -> Option<String> {
        if value.is_null() {
            return None;
        }
        let mut bytes = [0u8; 512];
        if unsafe { CFStringGetCString(value, bytes.as_mut_ptr().cast(), 512, 0x08000100) } == 0 {
            return None;
        }
        let length = bytes.iter().position(|b| *b == 0)?;
        String::from_utf8(bytes[..length].to_vec()).ok()
    }
    unsafe {
        let id = string(&format!("{locale}@calendar=gregorian"))?;
        let locale = owned(CFLocaleCreate(std::ptr::null(), id.0))?;
        let formatter = owned(CFDateFormatterCreate(std::ptr::null(), locale.0, 1, 0))?;
        let months = owned(CFDateFormatterCopyProperty(
            formatter.0,
            kCFDateFormatterMonthSymbols,
        ))?;
        let weekdays = owned(CFDateFormatterCopyProperty(
            formatter.0,
            kCFDateFormatterShortWeekdaySymbols,
        ))?;
        if CFArrayGetCount(months.0) != 12 || CFArrayGetCount(weekdays.0) != 7 {
            return None;
        }
        let months = (0..12)
            .map(|index| text(CFArrayGetValueAtIndex(months.0, index)))
            .collect::<Option<Vec<_>>>()?
            .try_into()
            .ok()?;
        let weekdays = (0..7)
            .map(|index| text(CFArrayGetValueAtIndex(weekdays.0, index)))
            .collect::<Option<Vec<_>>>()?
            .try_into()
            .ok()?;
        let calendar = owned(CFCalendarCreateWithIdentifier(
            std::ptr::null(),
            kCFGregorianCalendar,
        ))?;
        CFCalendarSetLocale(calendar.0, locale.0);
        let first_weekday = (CFCalendarGetFirstWeekday(calendar.0) - 1).clamp(0, 6) as u8;
        let template = string("yMd")?;
        let pattern = owned(CFDateFormatterCreateDateFormatFromTemplate(
            std::ptr::null(),
            template.0,
            0,
            locale.0,
        ))?;
        let (date_order, separators) = date_pattern(&text(pattern.0)?)?;
        let numbers = owned(CFNumberFormatterCreate(std::ptr::null(), locale.0, 1))?;
        let mut digits = ['0'; 10];
        for digit in 0..10i32 {
            let formatted = owned(CFNumberFormatterCreateStringWithValue(
                std::ptr::null(),
                numbers.0,
                3,
                (&digit as *const i32).cast(),
            ))?;
            let value = text(formatted.0)?;
            let mut chars = value
                .chars()
                .filter(|c| !matches!(c, '\u{200e}' | '\u{200f}' | '\u{061c}'));
            digits[digit as usize] = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
        }
        let profile = oryn_dom::ControlLocale {
            months,
            weekdays,
            first_weekday,
            date_order,
            separators,
            digits,
        };
        profile.valid().then_some(profile)
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(super) fn profile(_locale: &str) -> Option<oryn_dom::ControlLocale> {
    None
}
#[cfg(target_os = "windows")]
pub(super) fn profile(locale: &str) -> Option<oryn_dom::ControlLocale> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocaleInfoEx(locale: *const u16, kind: u32, buffer: *mut u16, count: i32) -> i32;
    }
    let locale = locale
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let get = |kind: u32| {
        let mut buffer = [0u16; 256];
        let length = unsafe { GetLocaleInfoEx(locale.as_ptr(), kind, buffer.as_mut_ptr(), 256) };
        if length <= 1 {
            None
        } else {
            String::from_utf16(&buffer[..length as usize - 1]).ok()
        }
    };
    let months = (0..12)
        .map(|index| get(0x38 + index))
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()?;
    let weekdays = (0..7)
        .map(|index| get(0x31 + (index + 6) % 7))
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()?;
    let first_weekday = (get(0x100c)?.parse::<u8>().ok()? + 1) % 7;
    let (date_order, separators) = date_pattern(&get(0x1f)?)?;
    let digits = get(0x13)?.chars().collect::<Vec<_>>().try_into().ok()?;
    let profile = oryn_dom::ControlLocale {
        months,
        weekdays,
        first_weekday,
        date_order,
        separators,
        digits,
    };
    profile.valid().then_some(profile)
}
fn date_pattern(pattern: &str) -> Option<([u8; 3], [String; 2])> {
    let mut fields = Vec::new();
    let mut separators = Vec::new();
    let mut between = String::new();
    let mut quoted = false;
    for c in pattern.chars() {
        if c == '\'' {
            quoted = !quoted;
            continue;
        }
        let field = if quoted {
            None
        } else {
            match c {
                'y' | 'Y' | 'u' => Some(0),
                'M' | 'L' => Some(1),
                'd' => Some(2),
                _ => None,
            }
        };
        if let Some(field) = field {
            if fields.last() == Some(&field) {
                continue;
            }
            if fields.contains(&field) {
                return None;
            }
            if !fields.is_empty() {
                separators.push(std::mem::take(&mut between));
            }
            fields.push(field);
        } else if !fields.is_empty() {
            between.push(c);
        }
    }
    Some((fields.try_into().ok()?, separators.try_into().ok()?))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_date_patterns_keep_order_and_unicode_separators() {
        assert_eq!(
            date_pattern("dd.MM.yyyy"),
            Some(([2, 1, 0], [".".into(), ".".into()]))
        );
        assert_eq!(
            date_pattern("y年M月d日"),
            Some(([0, 1, 2], ["年".into(), "月".into()]))
        );
        assert!(date_pattern("time only").is_none());
    }
}
