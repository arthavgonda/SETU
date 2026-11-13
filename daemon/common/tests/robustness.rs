use setu_common::engine::read_packet;
use setu_common::frame::{encode, parse_object, read_frame};
use setu_common::protocol::parse_envelope;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

const SEEDS: &[&str] = &[
    r#"{"v":1,"id":"c-1","type":"request","method":"hello","params":{"protocol":1}}"#,
    r#"{"v":1,"id":"c-2","type":"response","ok":true,"result":{}}"#,
    r#"{"v":1,"type":"event","topic":"audio.level","data":{"rms":0.1}}"#,
    r#"{"type":"request","method":"settings.set","params":{"patch":{"tts.rate":1.5}}}"#,
];

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for round in 0..20_000 {
        let n = (rng.next() % 512) as usize;
        let mut data = rng.bytes(n);
        if round % 3 == 0 && data.len() >= 4 {
            let len = (rng.next() % 600) as u32;
            data[..4].copy_from_slice(&len.to_be_bytes());
        }
        let mut cursor = std::io::Cursor::new(&data);
        let mut buf = Vec::new();
        while read_frame(&mut cursor, &mut buf).is_ok() {
            if let Ok(m) = parse_object(&buf) {
                let _ = parse_envelope(m);
            }
        }
        let mut cursor = std::io::Cursor::new(&data);
        for _ in 0..8 {
            if read_packet(&mut cursor).is_err() {
                break;
            }
        }
    }
}

#[test]
fn mutated_envelopes_never_panic() {
    let mut rng = Rng(42);
    for seed in SEEDS {
        for _ in 0..5_000 {
            let mut b = seed.as_bytes().to_vec();
            for _ in 0..(1 + rng.next() % 4) {
                let i = (rng.next() as usize) % b.len();
                b[i] = rng.next() as u8;
            }
            if let Ok(m) = parse_object(&b) {
                let _ = parse_envelope(m);
            }
            if let Ok(f) = encode(&b) {
                let mut cursor = std::io::Cursor::new(f);
                let mut buf = Vec::new();
                assert!(read_frame(&mut cursor, &mut buf).is_ok());
                assert_eq!(buf, b);
            }
        }
    }
}

#[test]
fn oversized_and_empty_frames_are_rejected() {
    let mut buf = Vec::new();
    let zero = 0u32.to_be_bytes();
    assert!(read_frame(&mut std::io::Cursor::new(&zero[..]), &mut buf).is_err());
    let huge = (2u32 * 1024 * 1024).to_be_bytes();
    assert!(read_frame(&mut std::io::Cursor::new(&huge[..]), &mut buf).is_err());
    assert!(encode(&vec![b'x'; 1024 * 1024 + 1]).is_err());
}
