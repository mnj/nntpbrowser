//! First selection fetches a bounded initial cache, independent of UI filters.

use super::{Event, Worker};

const INITIAL_HEADERS: u64 = 10_000;

impl Worker {
    pub(super) async fn ensure_group_headers(&mut self, group_id: i64) {
        match self.needs_initial_headers(group_id) {
            Ok(true) => {
                // Publish activity before connecting/authenticating/GROUP. The
                // actual retained range becomes known in the normal fetch flow.
                let _ = self.evt.send(Event::HeaderProgress {
                    group_id,
                    received: 0,
                    stored: 0,
                    completed: 0,
                    total: 0,
                    first: 0,
                    last: 0,
                });
                self.fetch_headers(group_id, Some(INITIAL_HEADERS)).await;
            }
            Ok(false) => {}
            Err(error) => {
                self.notice(error.clone());
                let _ = self.evt.send(Event::HeaderFetchFinished {
                    group_id,
                    error: Some(error),
                });
            }
        }
    }

    fn needs_initial_headers(&self, group_id: i64) -> Result<bool, String> {
        let group = self
            .catalog
            .group_by_id(group_id)
            .map_err(|error| format!("initial headers: {error}"))?
            .filter(|group| group.subscribed)
            .ok_or("initial headers: group is not subscribed")?;
        let ranges = self
            .headers
            .stored_ranges(group.server_id, &group.name)
            .map_err(|error| format!("initial header ranges: {error}"))?;
        Ok(ranges.is_empty())
    }
}
