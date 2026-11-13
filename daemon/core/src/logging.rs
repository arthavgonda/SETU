use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_BYTES: u64 = 10 * 1024 * 1024;
const KEEP: usize = 5;

struct Sink {
    file: Option<File>,
    written: u64,
}

pub struct Logger {
    dir: PathBuf,
    sink: Mutex<Sink>,
    stderr: bool,
}

impl Logger {
    fn path(&self, n: usize) -> PathBuf {
        if n == 0 { self.dir.join("setu-daemon.log") } else { self.dir.join(format!("setu-daemon.{n}.log")) }
    }

    fn rotate(&self, s: &mut Sink) {
        s.file = None;
        for n in (0..KEEP - 1).rev() {
            let _ = std::fs::rename(self.path(n), self.path(n + 1));
        }
        s.file = OpenOptions::new().create(true).append(true).open(self.path(0)).ok();
        s.written = 0;
    }
}

impl Log for Logger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= log::max_level()
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let ts = jiff::Timestamp::now();
        let line = format!(
            "{} {:<5} {} {}\n",
            ts.strftime("%Y-%m-%dT%H:%M:%S%.3fZ"),
            r.level(),
            r.target().trim_start_matches("setu_daemon::"),
            r.args()
        );
        if self.stderr {
            eprint!("{line}");
        }
        let Ok(mut s) = self.sink.lock() else { return };
        if s.written + line.len() as u64 > MAX_BYTES {
            self.rotate(&mut s);
        }
        if let Some(f) = s.file.as_mut()
            && f.write_all(line.as_bytes()).is_ok() {
                s.written += line.len() as u64;
            }
    }

    fn flush(&self) {
        if let Ok(mut s) = self.sink.lock()
            && let Some(f) = s.file.as_mut() {
                let _ = f.flush();
            }
    }
}

pub fn init(dir: PathBuf, stderr: bool) {
    let p = dir.join("setu-daemon.log");
    let written = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    let file = OpenOptions::new().create(true).append(true).open(&p).ok();
    let level = match std::env::var("SETU_LOG").as_deref() {
        Ok("debug") => LevelFilter::Debug,
        Ok("trace") => LevelFilter::Trace,
        Ok("warn") => LevelFilter::Warn,
        _ => LevelFilter::Info,
    };
    let logger = Box::new(Logger { dir, sink: Mutex::new(Sink { file, written }), stderr });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}

pub fn redact(text: &str) -> String {
    if log::max_level() >= Level::Trace {
        text.to_owned()
    } else {
        format!("[{} characters redacted]", text.chars().count())
    }
}
