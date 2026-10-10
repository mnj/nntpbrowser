//! Lifetime counters include idle sockets and short requests between samples.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::SemaphorePermit;

#[derive(Default)]
pub(crate) struct Metrics {
    active: AtomicUsize,
    peak: AtomicUsize,
    open: AtomicUsize,
}

impl Metrics {
    pub(super) fn lease<'a>(self: &Arc<Self>, permit: SemaphorePermit<'a>) -> Lease<'a> {
        let active = self.active.fetch_add(1, Ordering::Relaxed) + 1;
        self.peak.fetch_max(active, Ordering::Relaxed);
        Lease {
            _busy: Busy(self.clone()),
            _permit: permit,
        }
    }

    pub(crate) fn socket(self: &Arc<Self>) -> Socket {
        self.open.fetch_add(1, Ordering::Relaxed);
        Socket(self.clone())
    }

    pub(super) fn active(&self) -> usize {
        self.active.load(Ordering::Relaxed)
    }

    pub(super) fn take_peak(&self) -> usize {
        self.peak.swap(0, Ordering::Relaxed).max(self.active())
    }

    pub(super) fn open(&self) -> usize {
        self.open.load(Ordering::Relaxed)
    }
}

pub(super) struct Lease<'a> {
    _busy: Busy,
    _permit: SemaphorePermit<'a>,
}

struct Busy(Arc<Metrics>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(crate) struct Socket(Arc<Metrics>);

impl Drop for Socket {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::Relaxed);
    }
}
