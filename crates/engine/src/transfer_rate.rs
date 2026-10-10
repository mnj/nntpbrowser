//! Elapsed-time-weighted transfer rates with a five-second trailing window.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(5);
const MAX_SAMPLES: usize = 256;

#[derive(Debug, Clone)]
struct Sample {
    at: Instant,
    bytes: u64,
}

/// A bounded history of a cumulative byte counter.
///
/// Rates always use a five-second denominator, including startup time before
/// the first observation. Bytes between observations are weighted uniformly
/// over their elapsed interval; an idle rate expires without further samples.
/// At unusually high sampling frequencies, older intervals are coalesced while
/// retaining their elapsed time and total bytes.
#[derive(Debug, Clone)]
pub struct TransferRate {
    samples: VecDeque<Sample>,
}

impl TransferRate {
    pub fn new(total_bytes: u64, now: Instant) -> Self {
        Self {
            samples: VecDeque::from([Sample {
                at: now,
                bytes: total_bytes,
            }]),
        }
    }

    pub fn observe(&mut self, total_bytes: u64, now: Instant) {
        let last = self.samples.back().expect("rate baseline");
        // Reject old samples before interpreting a lower counter as a reset.
        if now < last.at {
            return;
        }
        if total_bytes < last.bytes {
            *self = Self::new(total_bytes, now);
            return;
        }
        // An out-of-order observation must not consume bytes that a later,
        // correctly timed sample still needs to account for.
        if now == last.at && total_bytes == last.bytes {
            return;
        }
        self.samples.push_back(Sample {
            at: now,
            bytes: total_bytes,
        });
        if let Some(cutoff) = now.checked_sub(WINDOW) {
            while self.samples.len() > 1 && self.samples[1].at <= cutoff {
                self.samples.pop_front();
            }
        }
        while self.samples.len() > MAX_SAMPLES {
            self.samples.remove(1);
        }
    }

    pub fn bytes_per_second(&self, now: Instant) -> f64 {
        let cutoff = now.checked_sub(WINDOW);
        let mut bytes = 0.0;
        for (first, last) in self.samples.iter().zip(self.samples.iter().skip(1)) {
            let delta = (last.bytes - first.bytes) as f64;
            if last.at == first.at {
                if last.at <= now && cutoff.is_none_or(|cutoff| last.at > cutoff) {
                    bytes += delta;
                }
                continue;
            }
            let start = cutoff.map_or(first.at, |cutoff| first.at.max(cutoff));
            let end = last.at.min(now);
            if let Some(overlap) = end.checked_duration_since(start) {
                bytes +=
                    delta * overlap.as_secs_f64() / last.at.duration_since(first.at).as_secs_f64();
            }
        }
        bytes / WINDOW.as_secs_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }
    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }

    #[test]
    fn sustained_traffic_uses_startup_zero_then_the_full_trailing_window() {
        let start = Instant::now();
        let mut rate = TransferRate::new(100, start);
        for seconds in 1..=10 {
            let now = at(start, seconds * 1000);
            rate.observe(100 + seconds * 1000, now);
            close(rate.bytes_per_second(now), (seconds.min(5) * 200) as f64);
        }
    }

    #[test]
    fn alternating_bursts_do_not_drop_to_zero_between_samples_and_expire_when_idle() {
        let start = Instant::now();
        let mut rate = TransferRate::new(0, start);
        let mut total = 0;
        for sample in 1..=40 {
            if sample % 2 == 1 {
                total += 1000;
            }
            let now = at(start, sample * 250);
            rate.observe(total, now);
            if sample >= 20 {
                close(rate.bytes_per_second(now), 2000.0);
            }
        }
        assert!(rate.bytes_per_second(at(start, 12_000)) > 0.0);
        close(rate.bytes_per_second(at(start, 15_000)), 0.0);
        close(rate.bytes_per_second(at(start, 25_000)), 0.0);
    }

    #[test]
    fn irregular_sampling_weights_only_the_elapsed_overlap() {
        let start = Instant::now();
        let mut rate = TransferRate::new(0, start);
        rate.observe(1000, at(start, 1000));
        rate.observe(6000, at(start, 6000));
        close(rate.bytes_per_second(at(start, 6000)), 1000.0);
        close(rate.bytes_per_second(at(start, 8000)), 600.0);
        rate.observe(6000, at(start, 10_000));
        close(rate.bytes_per_second(at(start, 10_000)), 200.0);
        close(rate.bytes_per_second(at(start, 11_000)), 0.0);
    }

    #[test]
    fn rollback_and_out_of_order_times_are_safe_and_do_not_create_spikes() {
        let start = Instant::now();
        let mut rate = TransferRate::new(0, start);
        rate.observe(1000, at(start, 1000));
        rate.observe(100, start); // An older, lower counter is not a reset.
        close(rate.bytes_per_second(at(start, 1000)), 200.0);
        rate.observe(1500, start); // Ignore this observation, including its counter.
        rate.observe(2000, at(start, 2000));
        close(rate.bytes_per_second(at(start, 2000)), 400.0);
        close(rate.bytes_per_second(start - Duration::from_secs(1)), 0.0);
        rate.observe(10, at(start, 3000));
        close(rate.bytes_per_second(at(start, 3000)), 0.0);
        rate.observe(510, at(start, 4000));
        close(rate.bytes_per_second(at(start, 4000)), 100.0);
        rate.observe(1010, at(start, 4000)); // Same-time byte burst.
        close(rate.bytes_per_second(at(start, 4000)), 200.0);
        close(rate.bytes_per_second(at(start, 9000)), 0.0);
    }

    #[test]
    fn high_frequency_samples_remain_bounded_and_preserve_sustained_rate() {
        let start = Instant::now();
        let mut rate = TransferRate::new(0, start);
        for sample in 1..=10_000 {
            rate.observe(sample, at(start, sample));
        }
        assert!(rate.samples.len() <= MAX_SAMPLES);
        close(rate.bytes_per_second(at(start, 10_000)), 1000.0);
    }
}
