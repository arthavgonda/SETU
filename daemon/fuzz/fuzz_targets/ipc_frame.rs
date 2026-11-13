#![no_main]

use libfuzzer_sys::fuzz_target;
use setu_common::frame::{parse_object, read_frame};
use setu_common::protocol::parse_envelope;

fuzz_target!(|data: &[u8]| {
    let mut cursor = std::io::Cursor::new(data);
    let mut buf = Vec::new();
    while read_frame(&mut cursor, &mut buf).is_ok() {
        if let Ok(m) = parse_object(&buf) {
            let _ = parse_envelope(m);
        }
    }
    if let Ok(m) = parse_object(data) {
        let _ = parse_envelope(m);
    }
});
