#[cfg(loom)]
use loom::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

use std::sync::Arc;

struct Shared {
    cells: Box<[AtomicU32]>,
    mask: usize,
    head: AtomicUsize,
    tail: AtomicUsize,
    dropped: AtomicU64,
}

pub struct Producer {
    s: Arc<Shared>,
}

pub struct Consumer {
    s: Arc<Shared>,
}

pub fn ring(capacity: usize) -> (Producer, Consumer) {
    let cap = capacity.next_power_of_two().max(2);
    let cells = (0..cap).map(|_| AtomicU32::new(0)).collect::<Vec<_>>().into_boxed_slice();
    let s = Arc::new(Shared {
        cells,
        mask: cap - 1,
        head: AtomicUsize::new(0),
        tail: AtomicUsize::new(0),
        dropped: AtomicU64::new(0),
    });
    (Producer { s: s.clone() }, Consumer { s })
}

impl Producer {
    pub fn push(&mut self, data: &[f32]) {
        let s = &*self.s;
        let cap = s.mask + 1;
        let data = if data.len() > cap {
            s.dropped.fetch_add((data.len() - cap) as u64, Ordering::Relaxed);
            &data[data.len() - cap..]
        } else {
            data
        };
        let n = data.len();
        let head = s.head.load(Ordering::Relaxed);
        let mut tail = s.tail.load(Ordering::Acquire);
        loop {
            let free = cap - head.wrapping_sub(tail);
            if free >= n {
                break;
            }
            let need = n - free;
            match s.tail.compare_exchange_weak(tail, tail.wrapping_add(need), Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    s.dropped.fetch_add(need as u64, Ordering::Relaxed);
                    break;
                }
                Err(t) => tail = t,
            }
        }
        for (i, v) in data.iter().enumerate() {
            s.cells[head.wrapping_add(i) & s.mask].store(v.to_bits(), Ordering::Relaxed);
        }
        s.head.store(head.wrapping_add(n), Ordering::Release);
    }

    pub fn free(&self) -> usize {
        let s = &*self.s;
        s.mask + 1 - s.head.load(Ordering::Relaxed).wrapping_sub(s.tail.load(Ordering::Acquire))
    }
}

impl Consumer {
    pub fn pop(&mut self, out: &mut Vec<f32>, max: usize) -> usize {
        let s = &*self.s;
        let start = out.len();
        loop {
            let tail = s.tail.load(Ordering::Acquire);
            let head = s.head.load(Ordering::Acquire);
            let n = head.wrapping_sub(tail).min(max);
            if n == 0 {
                return 0;
            }
            out.truncate(start);
            out.reserve(n);
            for i in 0..n {
                out.push(f32::from_bits(s.cells[tail.wrapping_add(i) & s.mask].load(Ordering::Relaxed)));
            }
            if s.tail.compare_exchange(tail, tail.wrapping_add(n), Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return n;
            }
        }
    }

    pub fn len(&self) -> usize {
        let s = &*self.s;
        s.head.load(Ordering::Acquire).wrapping_sub(s.tail.load(Ordering::Acquire))
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn dropped(&self) -> u64 {
        self.s.dropped.load(Ordering::Relaxed)
    }

    pub fn capacity(&self) -> usize {
        self.s.mask + 1
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn push_pop_wraps() {
        let (mut p, mut c) = ring(8);
        p.push(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let mut out = Vec::new();
        assert_eq!(c.pop(&mut out, 4), 4);
        assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
        p.push(&[7.0, 8.0, 9.0, 10.0, 11.0, 12.0]);
        out.clear();
        c.pop(&mut out, 100);
        assert_eq!(out, vec![5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0]);
    }

    #[test]
    fn full_ring_drops_oldest_and_counts() {
        let (mut p, mut c) = ring(4);
        p.push(&[1.0, 2.0, 3.0]);
        p.push(&[4.0, 5.0, 6.0]);
        assert_eq!(c.dropped(), 2);
        let mut out = Vec::new();
        c.pop(&mut out, 10);
        assert_eq!(out, vec![3.0, 4.0, 5.0, 6.0]);
        p.push(&[0.0; 9]);
        assert_eq!(c.dropped(), 7);
        assert_eq!(c.len(), 4);
    }

    #[test]
    fn threads_preserve_order_without_drops() {
        let (mut p, mut c) = ring(1024);
        let total = 200_000usize;
        let t = std::thread::spawn(move || {
            let mut next = 0usize;
            while next < total {
                let end = (next + 97).min(total);
                while p.free() < end - next {
                    std::hint::spin_loop();
                }
                let chunk: Vec<f32> = (next..end).map(|v| v as f32).collect();
                p.push(&chunk);
                next = end;
            }
        });
        let mut got = 0usize;
        let mut out = Vec::new();
        while got < total {
            out.clear();
            c.pop(&mut out, 512);
            for v in &out {
                assert_eq!(*v as usize, got);
                got += 1;
            }
        }
        t.join().unwrap();
        assert_eq!(c.dropped(), 0);
    }

    #[test]
    fn overrun_keeps_increasing_sequence() {
        let (mut p, mut c) = ring(256);
        let total = 400_000usize;
        let t = std::thread::spawn(move || {
            let mut next = 0usize;
            while next < total {
                let end = (next + 64).min(total);
                let chunk: Vec<f32> = (next..end).map(|v| v as f32).collect();
                p.push(&chunk);
                next = end;
            }
        });
        let mut last = -1f64;
        let mut out = Vec::new();
        let mut seen = 0usize;
        loop {
            out.clear();
            if c.pop(&mut out, 100) == 0 {
                if t.is_finished() && c.is_empty() {
                    break;
                }
                continue;
            }
            for v in &out {
                assert!((*v as f64) > last, "{v} after {last}");
                last = *v as f64;
                seen += 1;
            }
        }
        t.join().unwrap();
        assert_eq!(seen as u64 + c.dropped(), total as u64);
    }
}

#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;

    #[test]
    fn overwrite_ring_is_linearizable() {
        loom::model(|| {
            let (mut p, mut c) = ring(2);
            let t = loom::thread::spawn(move || {
                p.push(&[1.0]);
                p.push(&[2.0]);
                p.push(&[3.0]);
            });
            let mut out = Vec::new();
            c.pop(&mut out, 2);
            t.join().unwrap();
            c.pop(&mut out, 2);
            assert!(out.windows(2).all(|w| w[0] < w[1]));
            assert_eq!(out.len() as u64 + c.dropped(), 3);
            assert_eq!(*out.last().unwrap(), 3.0);
        });
    }
}
