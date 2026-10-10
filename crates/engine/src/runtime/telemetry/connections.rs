//! Map live protocol snapshots to bounded, smoothed per-connection UI samples.
use crate::{ConnectionStats, TransferRate};
use nntp::{ConnectionSnapshot, TrafficSnapshot};
use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

struct Rates {
    received: TransferRate,
    sent: TransferRate,
    last: TrafficSnapshot,
}

pub(super) struct Tracker {
    sampled_at: Instant,
    rates: HashMap<u64, Rates>,
}

impl Tracker {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            sampled_at: now,
            rates: HashMap::new(),
        }
    }

    pub(super) fn sample(
        &mut self,
        connections: Vec<ConnectionSnapshot>,
        now: Instant,
    ) -> Vec<ConnectionStats> {
        let live = connections
            .iter()
            .map(|connection| connection.id)
            .collect::<HashSet<_>>();
        self.rates.retain(|id, _| live.contains(id));
        let mut stats = Vec::with_capacity(connections.len());
        for connection in connections {
            let rates = self.rates.entry(connection.id).or_insert_with(|| Rates {
                received: TransferRate::new(0, self.sampled_at),
                sent: TransferRate::new(0, self.sampled_at),
                last: TrafficSnapshot::default(),
            });
            let traffic = connection.traffic;
            rates.received.observe(traffic.received, now);
            rates.sent.observe(traffic.sent, now);
            stats.push(ConnectionStats {
                id: connection.id,
                authenticated: connection.authenticated,
                read_pending: connection.read_pending,
                write_pending: connection.write_pending,
                receiving: traffic.received > rates.last.received,
                sending: traffic.sent > rates.last.sent,
                received_bytes: traffic.received,
                sent_bytes: traffic.sent,
                received_per_second: rates.received.bytes_per_second(now),
                sent_per_second: rates.sent.bytes_per_second(now),
            });
            rates.last = traffic;
        }
        self.sampled_at = self.sampled_at.max(now);
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn snapshot(id: u64, received: u64, sent: u64) -> ConnectionSnapshot {
        ConnectionSnapshot {
            id,
            authenticated: true,
            read_pending: false,
            write_pending: false,
            traffic: TrafficSnapshot {
                received,
                sent,
                decoded_received: received,
            },
        }
    }

    #[test]
    fn snapshots_preserve_state_and_short_bursts_while_rates_expire_when_idle() {
        let start = Instant::now();
        let mut tracker = Tracker::new(start);
        let mut connection = snapshot(7, 1000, 100);
        connection.read_pending = true;
        let first = tracker.sample(vec![connection], start + Duration::from_millis(250));
        assert_eq!(first[0].id, 7);
        assert!(first[0].authenticated && first[0].read_pending);
        assert!(!first[0].write_pending);
        assert!(first[0].receiving && first[0].sending);
        assert_eq!(first[0].received_per_second, 200.0);
        assert_eq!(first[0].sent_per_second, 20.0);
        let idle = tracker.sample(
            vec![snapshot(7, 1000, 100)],
            start + Duration::from_millis(500),
        );
        assert!(!idle[0].receiving && !idle[0].sending && !idle[0].read_pending);
        assert_eq!(idle[0].received_per_second, 200.0);
        let expired = tracker.sample(vec![snapshot(7, 1000, 100)], start + Duration::from_secs(6));
        assert_eq!(expired[0].received_per_second, 0.0);
        assert_eq!(expired[0].sent_per_second, 0.0);
    }

    #[test]
    fn closed_ids_are_pruned_and_new_connections_do_not_inherit_previous_rates() {
        let start = Instant::now();
        let mut tracker = Tracker::new(start);
        tracker.sample(
            vec![snapshot(1, 5000, 500), snapshot(2, 1000, 100)],
            start + Duration::from_millis(250),
        );
        let next = tracker.sample(
            vec![snapshot(2, 1000, 100), snapshot(3, 50, 25)],
            start + Duration::from_millis(500),
        );
        assert_eq!(tracker.rates.len(), 2);
        assert!(!tracker.rates.contains_key(&1));
        assert_eq!(next[1].received_per_second, 10.0);
        assert_eq!(next[1].sent_per_second, 5.0);
        assert!(tracker
            .sample(Vec::new(), start + Duration::from_millis(750))
            .is_empty());
        assert!(tracker.rates.is_empty());
    }
}
