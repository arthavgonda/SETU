#![no_main]

use libfuzzer_sys::fuzz_target;
use setu_common::engine::read_packet;

fuzz_target!(|data: &[u8]| {
    let mut cursor = std::io::Cursor::new(data);
    for _ in 0..64 {
        if read_packet(&mut cursor).is_err() {
            break;
        }
    }
});
