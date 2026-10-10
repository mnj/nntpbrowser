//! Shared cumulative bound for successful BODY payloads in parallel requests.

use crate::Error;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

/// A cloneable cumulative byte limit for the successful BODY payloads of one
/// operation. Clones share both the immutable limit and in-progress reservations.
#[derive(Clone, Debug)]
pub struct BodyBudget {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    limit: usize,
    used: AtomicUsize,
}

impl BodyBudget {
    /// One budget should belong to one bounded multipart operation. Completed
    /// bodies remain charged until the budget is dropped; failed or cancelled
    /// responses refund their partial bytes, allowing safe transport retries.
    pub fn new(limit: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                limit,
                used: AtomicUsize::new(0),
            }),
        }
    }

    pub fn limit(&self) -> usize {
        self.inner.limit
    }

    /// Successful dot-unstuffed BODY bytes plus in-progress reservations, including
    /// canonical CRLF per body line and excluding status lines and the terminator.
    pub fn bytes_used(&self) -> usize {
        self.inner.used.load(Ordering::Relaxed)
    }

    pub(crate) fn response(&self) -> Reservation<'_> {
        Reservation {
            budget: self,
            bytes: 0,
            committed: false,
        }
    }
}

pub(crate) struct Reservation<'a> {
    budget: &'a BodyBudget,
    bytes: usize,
    committed: bool,
}

impl Reservation<'_> {
    pub(crate) fn reserve(&mut self, bytes: usize) -> Result<(), Error> {
        // Retain fetch_update for compatibility with Rust 1.96.
        #[allow(deprecated)]
        self.budget
            .inner
            .used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes)
                    .filter(|&total| total <= self.budget.inner.limit)
            })
            .map_err(|_| Error::BodyTooLarge(self.budget.inner.limit))?;
        self.bytes += bytes;
        Ok(())
    }

    pub(crate) fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.budget
                .inner
                .used
                .fetch_sub(self.bytes, Ordering::Relaxed);
        }
    }
}
