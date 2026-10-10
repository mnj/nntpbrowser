//! Conservative, bounded header discovery; the caller owns persistence and controls.
use nntp::Pool;
use store::{HeaderStore, Record};

mod matcher;
mod names;
mod plan;
pub(super) use matcher::Discovery;
pub(super) use plan::DiscoveryPlan;

pub(super) const OVERVIEW_BATCH: u64 = 5_000;
pub(super) const SEARCH_BUDGET: u64 = 1_000_000;

/// Read an existing cache without publishing discovery results into it.
pub(super) fn scan_cached(
    headers: &HeaderStore,
    server_id: i64,
    group: &str,
    first: u64,
    last: u64,
    discovery: &mut Discovery,
) -> Result<(), String> {
    if first > last || last - first >= SEARCH_BUDGET {
        return Err("recovery cache scan exceeds its bounded vicinity".into());
    }
    let bad = headers
        .for_range(server_id, group, first, last, &mut |record| {
            discovery.ingest(record)
        })
        .map_err(|error| format!("recovery headers: {error}"))?;
    if !bad.is_empty() {
        return Err("recovery headers: cached overview blocks are damaged".into());
    }
    Ok(())
}

/// Select GROUP before this call. Dropping this future discards its checked-out
/// connection, so the caller can interrupt network waits on pause/cancel.
pub(super) async fn overview_batch(
    pool: &Pool,
    first: u64,
    last: u64,
) -> Result<Vec<Record>, String> {
    if first > last || last - first >= OVERVIEW_BATCH {
        return Err("recovery overview request exceeds the 5,000-article batch bound".into());
    }
    let mut records = Vec::new();
    let mut received = 0usize;
    let mut bytes = 0usize;
    let mut overflow = false;
    let (overflow_tx, overflow_rx) = tokio::sync::oneshot::channel();
    let mut overflow_tx = Some(overflow_tx);
    let fetch = pool.fetch_overview(first, last, |line| {
        received = received.saturating_add(1);
        bytes = bytes.saturating_add(
            line.subject.len() + line.from.len() + line.date.len() + line.message_id.len(),
        );
        if received > OVERVIEW_BATCH as usize || bytes > 8 * 1024 * 1024 {
            overflow = true;
            if let Some(sender) = overflow_tx.take() {
                let _ = sender.send(());
            }
            return;
        }
        if line.article < first || line.article > last {
            return;
        }
        let date_unix = store::parse_overview_date(&line.date).unwrap_or(0);
        records.push(Record {
            article: line.article,
            date_unix,
            date_raw: (date_unix == 0 && !line.date.is_empty()).then_some(line.date),
            bytes: line.bytes,
            lines: u32::try_from(line.lines).unwrap_or(u32::MAX),
            message_id: line.message_id,
            subject: line.subject,
            from: line.from,
        });
    });
    let result = {
        tokio::pin!(fetch);
        tokio::select! {
            result = &mut fetch => result,
            Ok(()) = overflow_rx => return Err("recovery overview response exceeded its bounded header batch".into()),
        }
    };
    match result {
        Ok(_) => {}
        Err(nntp::Error::Missing) if records.is_empty() => {}
        Err(error) => return Err(format!("recovery overview: {error}")),
    }
    if overflow {
        return Err("recovery overview response exceeded its bounded header batch".into());
    }
    Ok(records)
}

#[cfg(test)]
mod tests;
