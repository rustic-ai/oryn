#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(html) = std::str::from_utf8(data) {
        let document = oryn_native::html::parse(html);
        for (id, _) in document.dom.iter().take(512) {
            let _ = document.dom.text_content(id);
            let _ = document.dom.ancestors(id);
        }
    }
});
