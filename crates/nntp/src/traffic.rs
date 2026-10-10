//! Cumulative NNTP traffic, independent of a pool's socket lifetime.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Transport and decoded reader totals. Independent atomic counters are sampled
/// without pausing concurrent connections; this is not a synchronized ratio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrafficSnapshot {
    /// Actual NNTP bytes read, before compression decoding and after TLS.
    /// Includes setup, retries, failed responses and buffered read-ahead.
    pub received: u64,
    /// Actual successfully written NNTP bytes, after compression and before TLS.
    /// Includes setup and prefixes written before cancellation or failure.
    pub sent: u64,
    /// Consumed NNTP data after compression decoding, including status lines,
    /// line endings and dot framing. Encoded XZVER wrappers are replaced by their
    /// inflated payload. Buffered read-ahead is counted only when consumed.
    pub decoded_received: u64,
}

/// Shareable cumulative counters. Retaining this handle does not retain sockets.
#[derive(Default)]
pub struct Traffic {
    received: AtomicU64,
    sent: AtomicU64,
    decoded_received: AtomicU64,
    parent: Option<Arc<Traffic>>,
}

impl Traffic {
    pub(crate) fn with_parent(parent: Arc<Traffic>) -> Self {
        Self {
            parent: Some(parent),
            ..Self::default()
        }
    }

    pub fn snapshot(&self) -> TrafficSnapshot {
        TrafficSnapshot {
            received: self.received.load(Ordering::Relaxed),
            sent: self.sent.load(Ordering::Relaxed),
            decoded_received: self.decoded_received.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn receive(&self, bytes: usize) {
        self.received.fetch_add(bytes as u64, Ordering::Relaxed);
        if let Some(parent) = &self.parent {
            parent.receive(bytes);
        }
    }

    pub(crate) fn send(&self, bytes: usize) {
        self.sent.fetch_add(bytes as u64, Ordering::Relaxed);
        if let Some(parent) = &self.parent {
            parent.send(bytes);
        }
    }

    pub(crate) fn decode(&self, bytes: usize) {
        self.decoded_received
            .fetch_add(bytes as u64, Ordering::Relaxed);
        if let Some(parent) = &self.parent {
            parent.decode(bytes);
        }
    }
}
