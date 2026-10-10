//! Bounded background work. Results retain input order; cancellation stops new work.
use std::sync::{atomic::{AtomicBool, AtomicUsize, Ordering}, mpsc};

pub const NETWORK_WORKERS: usize = 4;

pub fn map_bounded<T: Sync, R: Send>(
    items: &[T], workers: usize, cancelled: &AtomicBool,
    work: impl Fn(&T) -> R + Sync, mut completed: impl FnMut(usize, &R),
) -> Vec<(usize, R)> {
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let (send, receive) = mpsc::channel();
        for _ in 0..workers.clamp(1, NETWORK_WORKERS).min(items.len()) {
            let send = send.clone(); let next = &next; let work = &work;
            scope.spawn(move || loop {
                if cancelled.load(Ordering::Relaxed) { break; }
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(index) else { break; };
                if cancelled.load(Ordering::Relaxed) { break; }
                if send.send((index, work(item))).is_err() { break; }
            });
        }
        drop(send);
        let mut results = Vec::new();
        for (index, value) in receive { completed(index, &value); results.push((index, value)); }
        results.sort_by_key(|(index, _)| *index);
        results
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    #[test]
    fn overlaps_four_jobs_and_retains_order() {
        let barrier = Barrier::new(4); let active = AtomicUsize::new(0); let maximum = AtomicUsize::new(0);
        let input: Vec<_> = (0..12).collect();
        let results = map_bounded(&input, 16, &AtomicBool::new(false), |&n| {
            let count = active.fetch_add(1, Ordering::SeqCst) + 1; maximum.fetch_max(count, Ordering::SeqCst);
            if n < 4 { barrier.wait(); }
            active.fetch_sub(1, Ordering::SeqCst); n * 2
        }, |_, _| {});
        assert_eq!(maximum.load(Ordering::SeqCst), 4);
        assert_eq!(results, input.iter().map(|&n| (n, n * 2)).collect::<Vec<_>>());
    }
    #[test]
    fn cancellation_keeps_finished_work_and_stops_dispatch() {
        let cancelled = AtomicBool::new(false);
        let results = map_bounded(&[0, 1, 2, 3], 1, &cancelled, |&n| { cancelled.store(true, Ordering::SeqCst); n }, |_, _| {});
        assert_eq!(results, vec![(0, 0)]);
        assert!(map_bounded(&[1], 4, &cancelled, |_| panic!("cancelled"), |_, _| {}).is_empty());
    }
}
