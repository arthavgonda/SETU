use crate::engines::Engine;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

const MAX: usize = 16_000 * 30;

#[derive(Default)]
struct State {
    id: u64,
    ready: bool,
    open: bool,
    pending: Vec<Vec<f32>>,
    pending_len: usize,
    utterance: Vec<f32>,
    keep: bool,
}

pub struct AsrFeed {
    s: Mutex<State>,
    engine: Arc<Engine>,
    ready_tx: watch::Sender<u64>,
}

impl AsrFeed {
    pub fn new(engine: Arc<Engine>) -> Arc<Self> {
        Arc::new(Self { s: Mutex::new(State::default()), engine, ready_tx: watch::channel(0).0 })
    }

    pub fn open(&self, id: u64, keep_audio: bool) {
        let mut s = self.s.lock().unwrap();
        *s = State { id, open: true, keep: keep_audio, ..Default::default() };
    }

    pub fn ready(&self, id: u64) {
        let mut s = self.s.lock().unwrap();
        if s.id != id || !s.open {
            return;
        }
        s.ready = true;
        for chunk in std::mem::take(&mut s.pending) {
            self.engine.pcm(chunk);
        }
        s.pending_len = 0;
        drop(s);
        self.ready_tx.send_replace(id);
    }

    pub async fn wait_ready(&self, id: u64, limit: Duration) -> bool {
        let mut rx = self.ready_tx.subscribe();
        let check = |s: &State| s.id == id && s.ready;
        if check(&self.s.lock().unwrap()) {
            return true;
        }
        tokio::time::timeout(limit, async {
            loop {
                if rx.changed().await.is_err() {
                    return false;
                }
                if *rx.borrow() == id {
                    return true;
                }
            }
        })
        .await
        .unwrap_or(false)
    }

    pub fn is_open(&self, id: u64) -> bool {
        let s = self.s.lock().unwrap();
        s.id == id && s.open
    }

    pub fn push(&self, chunk: Vec<f32>) {
        let mut s = self.s.lock().unwrap();
        if !s.open {
            return;
        }
        if s.keep && s.utterance.len() + chunk.len() <= MAX {
            s.utterance.extend_from_slice(&chunk);
        }
        if s.ready {
            drop(s);
            self.engine.pcm(chunk);
        } else if s.pending_len + chunk.len() <= MAX {
            s.pending_len += chunk.len();
            s.pending.push(chunk);
        }
    }

    pub fn close(&self) -> Vec<f32> {
        let mut s = self.s.lock().unwrap();
        s.open = false;
        s.ready = false;
        s.pending.clear();
        s.pending_len = 0;
        std::mem::take(&mut s.utterance)
    }
}
