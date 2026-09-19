//! OS UI locale sampled in the browser parent, never from author JavaScript.
fn normalize(value: &str) -> Option<String> {
    let value = value.split(['.', '@']).next()?.trim().replace('_', "-");
    if value.len() > 128
        || value.is_empty()
        || matches!(value.as_str(), "C" | "POSIX")
        || !value.split('-').all(|part| {
            !part.is_empty()
                && part.len() <= 8
                && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
    {
        return None;
    }
    Some(value)
}
pub(crate) fn current() -> String {
    static LOCALE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LOCALE
        .get_or_init(|| {
            system()
                .and_then(|value| normalize(&value))
                .unwrap_or_else(|| "en-US".into())
        })
        .clone()
}
#[cfg(target_os = "macos")]
fn system() -> Option<String> {
    use std::ffi::{c_char, c_void};
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFLocaleCopyCurrent() -> *const c_void;
        fn CFLocaleGetIdentifier(locale: *const c_void) -> *const c_void;
        fn CFStringGetCString(
            value: *const c_void,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(value: *const c_void);
    }
    unsafe {
        let locale = CFLocaleCopyCurrent();
        if locale.is_null() {
            return None;
        }
        let identifier = CFLocaleGetIdentifier(locale);
        let mut bytes = [0u8; 256];
        let copied = !identifier.is_null()
            && CFStringGetCString(
                identifier,
                bytes.as_mut_ptr().cast(),
                bytes.len() as isize,
                0x08000100,
            ) != 0;
        CFRelease(locale);
        if !copied {
            return None;
        }
        let end = bytes.iter().position(|byte| *byte == 0)?;
        String::from_utf8(bytes[..end].to_vec()).ok()
    }
}
#[cfg(target_os = "windows")]
fn system() -> Option<String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(buffer: *mut u16, count: i32) -> i32;
    }
    let mut value = [0u16; 85];
    let length = unsafe { GetUserDefaultLocaleName(value.as_mut_ptr(), 85) };
    if length <= 1 {
        return None;
    }
    String::from_utf16(&value[..length as usize - 1]).ok()
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn system() -> Option<String> {
    ["LC_ALL", "LC_TIME", "LANG"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locale_normalization_keeps_region_and_rejects_malformed_native_data() {
        assert_eq!(normalize("de_DE.UTF-8"), Some("de-DE".into()));
        assert_eq!(normalize("zh_Hant_TW"), Some("zh-Hant-TW".into()));
        assert_eq!(normalize("C"), None);
        assert_eq!(normalize("en/US"), None);
    }
}
