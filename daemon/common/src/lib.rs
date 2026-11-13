pub mod engine;
pub mod frame;
pub mod meter;
pub mod protocol;

pub const PROTOCOL_MIN: u32 = 1;
pub const PROTOCOL_MAX: u32 = 1;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SAMPLE_RATE: u32 = 16_000;
