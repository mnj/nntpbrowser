//! Session traffic totals survive socket closure and pool replacement.
use super::*;
use engine::TrafficSnapshot;

#[derive(Default)]
pub(super) struct TrafficTotals {
    total: TrafficSnapshot,
    pools: HashMap<(i64, u64), TrafficSnapshot>,
}

impl TrafficTotals {
    pub(super) fn observe(&mut self, key: (i64, u64), next: TrafficSnapshot, closed: bool) {
        let previous = self.pools.remove(&key).unwrap_or_default();
        self.total.received = self
            .total
            .received
            .saturating_add(next.received.saturating_sub(previous.received));
        self.total.sent = self
            .total
            .sent
            .saturating_add(next.sent.saturating_sub(previous.sent));
        self.total.decoded_received = self.total.decoded_received.saturating_add(
            next.decoded_received
                .saturating_sub(previous.decoded_received),
        );
        if !closed {
            self.pools.insert(key, next);
        }
    }

    pub(super) fn tooltip(&self, speed: &str) -> String {
        let humanized = |bytes: u64| format_bytes(i64::try_from(bytes).unwrap_or(i64::MAX));
        format!(
            "{speed} · 5-second average\n\nSession traffic across all servers\nReceived (network): {}\nReceived after decompression: {}\nSent (network): {}\n\nNNTP bytes, excluding TLS overhead.\nDecompressed totals count consumed responses.",
            humanized(self.total.received),
            humanized(self.total.decoded_received),
            humanized(self.total.sent),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(received: u64, sent: u64, decoded_received: u64) -> TrafficSnapshot {
        TrafficSnapshot {
            received,
            sent,
            decoded_received,
        }
    }

    #[test]
    fn session_totals_count_deltas_and_keep_final_traffic_from_replaced_pools() {
        let mut traffic = TrafficTotals::default();
        traffic.observe((1, 1), snapshot(100, 10, 400), false);
        traffic.observe((1, 1), snapshot(100, 10, 400), false);
        traffic.observe((2, 2), snapshot(200, 20, 800), false);
        traffic.observe((1, 1), snapshot(150, 15, 600), true);
        assert!(!traffic.pools.contains_key(&(1, 1)));
        traffic.observe((1, 3), snapshot(25, 5, 100), false);
        assert_eq!(traffic.total.received, 375);
        assert_eq!(traffic.total.sent, 40);
        assert_eq!(traffic.total.decoded_received, 1500);
    }

    #[test]
    fn tooltip_humanizes_each_counter_and_names_the_measurement_window() {
        let mut traffic = TrafficTotals::default();
        traffic.observe(
            (1, 1),
            snapshot(2 * 1024 * 1024, 1024, 8 * 1024 * 1024),
            false,
        );
        let tooltip = traffic.tooltip("400 KiB/s");
        assert!(tooltip.contains("5-second average"));
        assert!(tooltip.contains("Received (network): 2.0 MB"));
        assert!(tooltip.contains("Received after decompression: 8.0 MB"));
        assert!(tooltip.contains("Sent (network): 1.0 KB"));
    }
}
