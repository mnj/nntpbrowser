//! Fetch bounded ordered article windows and assemble a single preview in RAM.
use super::{PreviewRequest, PreviewResult};
use crate::decode::same_attachment_name;
use crate::{decode, Event};
use nntp::{ArticleRef, BodyBudget, Pool};
use std::{collections::BTreeMap, sync::Arc};
mod window;

pub(super) async fn fetch_preview(
    request: PreviewRequest,
    pools: Vec<Arc<Pool>>,
    events: tokio::sync::mpsc::UnboundedSender<Event>,
) -> PreviewResult {
    let bytes = assemble_preview(&request, &pools, &events).await;
    PreviewResult { request, bytes }
}

async fn assemble_preview(
    request: &PreviewRequest,
    pools: &[Arc<Pool>],
    events: &tokio::sync::mpsc::UnboundedSender<Event>,
) -> Result<Arc<[u8]>, String> {
    if pools.is_empty() {
        return Err("no enabled servers configured".into());
    }
    let mut assembled = Vec::new();
    let mut spans = Vec::new();
    let mut declared = None;
    let mut received = 0u64;
    let mut uu_mode = None;
    let mut uu_bodies = BTreeMap::new();
    let encoded_cap = (request.cap * 2 + 65_536) as usize;
    let progress = |received, total| {
        let _ = events.send(Event::PreviewMediaProgress {
            post_id: request.post_id,
            generation: request.generation,
            received,
            total,
        });
    };
    progress(0, request.size);
    let connections = pools[0].max_connections();
    // Spread parts across available sockets first, then pipeline remaining parts.
    let window_count = request
        .segments
        .len()
        .min(connections)
        .max(request.segments.len().div_ceil(pools[0].pipeline_depth()));
    let base = request.segments.len() / window_count;
    let extra = request.segments.len() % window_count;
    let mut first = 0;
    let ranges = (0..window_count)
        .map(|index| {
            let end = first + base + usize::from(index < extra);
            let range = first..end;
            first = end;
            range
        })
        .collect::<Vec<_>>();
    let budget = BodyBudget::new(encoded_cap);
    let mut windows = ranges.into_iter().enumerate();
    let mut tasks = tokio::task::JoinSet::new();
    let mut ready = BTreeMap::new();
    loop {
        while tasks.len() < connections {
            let Some((index, range)) = windows.next() else {
                break;
            };
            let ids = request.segments[range]
                .iter()
                .map(|segment| ArticleRef::MessageId(segment.message_id.clone()))
                .collect();
            let pools = pools.to_vec();
            let budget = budget.clone();
            tasks.spawn(async move { window::fetch(index, ids, pools, budget, encoded_cap).await });
        }
        let Some(result) = tasks.join_next().await else {
            break;
        };
        let (index, bodies) = result.map_err(|error| format!("preview window: {error}"))??;
        ready.insert(index, bodies);
        // UU continuations lack offsets; inspect the original first body even
        // when a later window or fill server completes ahead of it.
        if uu_mode.is_none() {
            let Some(first) = ready
                .get(&0)
                .and_then(|bodies: &Vec<Vec<u8>>| bodies.first())
            else {
                continue;
            };
            uu_mode = Some(decode::is_uuencode_fragment(first));
        }
        for (index, bodies) in std::mem::take(&mut ready) {
            if uu_mode == Some(true) {
                uu_bodies.insert(index, bodies);
                continue;
            }
            for body in bodies {
                let part = decode::decode_body_limited(&body, request.cap as usize)
                    .map_err(|error| error.to_string())?;
                let end = part
                    .begin
                    .checked_add(part.data.len() as u64)
                    .ok_or("media part offset overflow")?;
                if end > request.cap
                    || received.saturating_add(part.data.len() as u64) > request.cap
                    || part
                        .file_size
                        .is_some_and(|size| size == 0 || size > request.cap || end > size)
                {
                    return Err("decoded media exceeds the preview limit".into());
                }
                if let Some(size) = part.file_size {
                    if declared.is_some_and(|prior| prior != size) {
                        return Err("media parts disagree about file size".into());
                    }
                    declared = Some(size);
                }
                if declared.is_some_and(|size| end > size) {
                    return Err("media part exceeds its declared file size".into());
                }
                validate_name(&part, request)?;
                if spans
                    .iter()
                    .any(|&(begin, prior_end)| part.begin < prior_end && end > begin)
                {
                    return Err("media parts overlap".into());
                }
                assembled.resize(assembled.len().max(declared.unwrap_or(end) as usize), 0);
                assembled[part.begin as usize..end as usize].copy_from_slice(&part.data);
                spans.push((part.begin, end));
                received += part.data.len() as u64;
                progress(received, declared.unwrap_or(request.size));
            }
        }
    }
    if uu_mode == Some(true) {
        let uu_bodies = uu_bodies.into_values().flatten().collect::<Vec<_>>();
        let articles = uu_bodies.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let part = decode::decode_uuencode_parts_limited(&articles, request.cap as usize)
            .map_err(|error| error.to_string())?;
        validate_name(&part, request)?;
        progress(part.data.len() as u64, part.data.len() as u64);
        return Ok(Arc::from(part.data));
    }
    spans.sort_unstable();
    let mut cursor = 0;
    for (begin, end) in spans {
        if begin != cursor {
            return Err("media parts have gaps".into());
        }
        cursor = end;
    }
    if cursor != assembled.len() as u64 {
        return Err("media file is incomplete".into());
    }
    progress(cursor, cursor);
    Ok(Arc::from(assembled))
}

fn validate_name(part: &decode::Part, request: &PreviewRequest) -> Result<(), String> {
    if part
        .filename
        .as_deref()
        .is_some_and(|name| !same_attachment_name(name, &request.identity.name))
    {
        return Err(format!(
            "media body filename {:?} does not match header filename {:?}",
            part.filename.as_deref().unwrap_or_default(),
            request.identity.name
        ));
    }
    Ok(())
}
