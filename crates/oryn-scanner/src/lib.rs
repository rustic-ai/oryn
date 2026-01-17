/// The Universal Scanner JavaScript implementation.
/// This string is injected into browser contexts by backends.
pub const SCANNER_JS: &str = include_str!("scanner.js");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::const_is_empty)]
    fn it_works() {
        assert!(!SCANNER_JS.is_empty());
        assert!(SCANNER_JS.contains("Oryn"));
    }
}
