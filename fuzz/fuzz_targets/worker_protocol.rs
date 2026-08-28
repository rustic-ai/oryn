#![no_main]

use std::io::{BufReader, Cursor};

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > oryn_native::worker::MAX_IPC_FRAME_BYTES {
        return;
    }
    let mut input = data.to_vec();
    input.push(b'\n');
    let mut output = Vec::new();
    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        let _ = runtime.block_on(oryn_native::worker::serve(
            BufReader::new(Cursor::new(input)),
            &mut output,
        ));
    }
});
