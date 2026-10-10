//! One FIFO window retaining original part order across missing-article fill.
use nntp::{ArticleRef, BodyBudget, Pool};
use std::sync::Arc;

pub(super) async fn fetch(
    index: usize,
    ids: Vec<ArticleRef>,
    pools: Vec<Arc<Pool>>,
    budget: BodyBudget,
    cap: usize,
) -> Result<(usize, Vec<Vec<u8>>), String> {
    let mut pending = ids.into_iter().enumerate().collect::<Vec<_>>();
    let mut bodies = vec![None; pending.len()];
    for pool in pools {
        if pending.is_empty() {
            break;
        }
        let ids = pending.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>();
        let limits = vec![cap; ids.len()];
        let replies = pool
            .body_many_budgeted(&ids, &limits, &budget)
            .await
            .map_err(|error| error.to_string())?;
        let mut missing = Vec::new();
        for ((part, id), reply) in pending.drain(..).zip(replies) {
            match reply {
                Ok(bytes) => bodies[part] = Some(bytes),
                Err(nntp::Error::Missing) => missing.push((part, id)),
                Err(error) => return Err(error.to_string()),
            }
        }
        pending = missing;
    }
    if !pending.is_empty() {
        return Err("media article is missing".into());
    }
    let bodies = bodies
        .into_iter()
        .map(|body| body.ok_or_else(|| "media article is missing".to_string()))
        .collect::<Result<_, _>>()?;
    Ok((index, bodies))
}
