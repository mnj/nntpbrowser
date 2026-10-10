//! Live connection state without retaining sockets or protocol contents.

use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

use crate::{Traffic, TrafficSnapshot};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConnectionSnapshot {
    /// Stable, never reused within this pool, including failed setup attempts.
    pub id: u64,
    /// Greeting, TLS policy and authentication have completed successfully.
    pub authenticated: bool,
    /// A network read is in progress, including waiting for response bytes.
    pub read_pending: bool,
    /// A network write or flush is in progress.
    pub write_pending: bool,
    pub traffic: TrafficSnapshot,
}

#[derive(Default)]
pub(crate) struct Registry {
    next: AtomicU64,
    entries: Mutex<BTreeMap<u64, Arc<State>>>,
}

impl Registry {
    pub(crate) fn register(self: &Arc<Self>, aggregate: Arc<Traffic>) -> Registration {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let state = Arc::new(State {
            traffic: Arc::new(Traffic::with_parent(aggregate)),
            authenticated: AtomicBool::new(false),
            read_pending: AtomicBool::new(false),
            write_pending: AtomicBool::new(false),
        });
        self.entries.lock().unwrap().insert(id, state.clone());
        Registration {
            id,
            registry: self.clone(),
            state,
        }
    }

    pub(crate) fn snapshot(&self) -> Vec<ConnectionSnapshot> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .map(|(id, state)| ConnectionSnapshot {
                id: *id,
                authenticated: state.authenticated.load(Ordering::Relaxed),
                read_pending: state.read_pending.load(Ordering::Relaxed),
                write_pending: state.write_pending.load(Ordering::Relaxed),
                traffic: state.traffic.snapshot(),
            })
            .collect()
    }
}

pub(crate) struct Registration {
    id: u64,
    registry: Arc<Registry>,
    pub(crate) state: Arc<State>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.registry.entries.lock().unwrap().remove(&self.id);
    }
}

pub(crate) struct State {
    pub(crate) traffic: Arc<Traffic>,
    authenticated: AtomicBool,
    read_pending: AtomicBool,
    write_pending: AtomicBool,
}

impl State {
    pub(crate) fn authenticate(&self) {
        self.authenticated.store(true, Ordering::Relaxed);
    }

    pub(crate) fn reading(self: &Arc<Self>) -> Pending {
        self.read_pending.store(true, Ordering::Relaxed);
        Pending {
            state: self.clone(),
            read: true,
        }
    }

    pub(crate) fn writing(self: &Arc<Self>) -> Pending {
        self.write_pending.store(true, Ordering::Relaxed);
        Pending {
            state: self.clone(),
            read: false,
        }
    }
}

pub(crate) struct Pending {
    state: Arc<State>,
    read: bool,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if self.read {
            self.state.read_pending.store(false, Ordering::Relaxed);
        } else {
            self.state.write_pending.store(false, Ordering::Relaxed);
        }
    }
}
