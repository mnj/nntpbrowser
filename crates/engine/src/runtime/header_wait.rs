//! Poll UI controls while a header connection is waiting on the network.
use super::{Command, Worker};
use std::{future::Future, time::Duration};
use tokio::sync::mpsc::UnboundedReceiver;

impl Worker {
    pub(super) fn cancel_header_fetch(&mut self, group_id: i64) {
        if self.active_header_fetch == Some(group_id) {
            self.header_fetch_cancelled = true;
        }
    }

    pub(super) fn route_header_cancel(&mut self, group_id: i64) {
        if self.active_header_fetch == Some(group_id) {
            self.cancel_header_fetch(group_id);
        } else if self
            .deferred
            .iter()
            .any(|command| starts_fetch(command, group_id))
        {
            // Controls can be polled during unrelated work. Preserve a cancel
            // that arrived after an already-deferred fetch instead of treating
            // it as an idle, stale cancel.
            self.deferred
                .push_back(Command::CancelHeaderFetch(group_id));
        }
    }

    pub(super) fn consume_deferred_header_cancel(&mut self, group_id: i64) {
        let mut index = 0;
        while index < self.deferred.len() {
            if starts_fetch(&self.deferred[index], group_id) {
                break;
            }
            if matches!(self.deferred[index], Command::CancelHeaderFetch(id) if id == group_id) {
                self.deferred.remove(index);
                self.cancel_header_fetch(group_id);
            } else {
                index += 1;
            }
        }
    }

    pub(super) async fn wait_header<T>(&mut self, future: impl Future<Output = T>) -> Option<T> {
        let (_sender, receiver) = tokio::sync::mpsc::unbounded_channel::<()>();
        self.wait_header_stream(future, receiver, |_, ()| {}).await
    }

    pub(super) async fn wait_header_stream<T, L>(
        &mut self,
        future: impl Future<Output = T>,
        mut lines: UnboundedReceiver<L>,
        mut on_line: impl FnMut(&mut Self, L),
    ) -> Option<T> {
        tokio::pin!(future);
        let mut controls = tokio::time::interval(Duration::from_millis(25));
        loop {
            tokio::select! {
                biased;
                _ = controls.tick() => {
                    self.poll_controls();
                    if self.header_fetch_cancelled {
                        // Dropping the network future discards its local connection
                        // and releases the pool permit; unread replies are never reused.
                        return None;
                    }
                }
                result = &mut future => {
                    let mut drained = 0usize;
                    while let Ok(line) = lines.try_recv() {
                        if drained.is_multiple_of(250) {
                            self.poll_controls();
                            if self.header_fetch_cancelled { return None; }
                        }
                        on_line(self, line);
                        drained += 1;
                    }
                    return Some(result);
                }
                Some(line) = lines.recv() => on_line(self, line),
            }
        }
    }
}

fn starts_fetch(command: &Command, group_id: i64) -> bool {
    matches!(command,
        Command::SyncGroup(id) | Command::EnsureGroupHeaders(id) if *id == group_id)
        || matches!(command, Command::FetchHeaders { group_id: id, .. } if *id == group_id)
}
