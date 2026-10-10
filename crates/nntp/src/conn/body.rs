//! Bounded article body reads. Oversize responses leave this connection unusable.

use super::*;
use crate::BodyBudget;

impl Connection {
    /// Stop reading when the unstuffed body exceeds `max_bytes`, without
    /// buffering the remainder. The caller must discard the connection on error.
    pub async fn body_limited(
        &mut self,
        id: &ArticleRef,
        max_bytes: usize,
    ) -> Result<Vec<u8>, Error> {
        let command = format!("BODY {}\r\n", checked_ref(id)?);
        trace_request("BODY");
        self.writer
            .write_bytes(command.as_bytes())
            .await
            .map_err(|err| err.timeout_context("BODY"))?;
        read_body_limited(&mut self.reader, max_bytes, None).await
    }

    /// FIFO pipelined replies, with an independent bound per article. Completed
    /// replies survive a later truncated/oversize reply; subsequent slots fail.
    pub async fn body_pipeline_limited(
        &mut self,
        ids: &[ArticleRef],
        limits: &[usize],
    ) -> Result<Vec<Result<Vec<u8>, Error>>, Error> {
        self.body_pipeline_with_budget(ids, limits, None).await
    }

    /// Pipeline with per-article limits and a cumulative budget shared by every
    /// parallel window of one operation. Budget overflow invalidates this socket.
    pub async fn body_pipeline_budgeted(
        &mut self,
        ids: &[ArticleRef],
        limits: &[usize],
        budget: &BodyBudget,
    ) -> Result<Vec<Result<Vec<u8>, Error>>, Error> {
        self.body_pipeline_with_budget(ids, limits, Some(budget))
            .await
    }

    pub(crate) async fn body_pipeline_with_budget(
        &mut self,
        ids: &[ArticleRef],
        limits: &[usize],
        budget: Option<&BodyBudget>,
    ) -> Result<Vec<Result<Vec<u8>, Error>>, Error> {
        if ids.len() != limits.len() {
            return Err(Error::Protocol {
                code: 0,
                text: "body limits do not match request count".into(),
            });
        }
        let commands = ids
            .iter()
            .map(|id| checked_ref(id).map(|id| format!("BODY {id}\r\n").into_bytes()))
            .collect::<Result<Vec<_>, _>>()?;
        for command in &commands {
            crate::codec::validate_command(command)?;
        }
        let reader = &mut self.reader;
        let writer = &mut self.writer;
        let read = async {
            let mut replies = Vec::with_capacity(limits.len());
            for &limit in limits {
                let reply = read_body_limited(reader, limit, budget).await;
                let broken = reply.as_ref().is_err_and(body_connection_broken);
                replies.push(reply);
                if broken {
                    replies.extend((replies.len()..limits.len()).map(|_| Err(Error::Closed)));
                    break;
                }
            }
            replies
        };
        let (write, replies) = tokio::join!(write_all_cmds(writer, &commands), read);
        if !replies.iter().any(Result::is_ok) {
            write?;
        }
        Ok(replies)
    }
}

pub(super) fn body_connection_broken(err: &Error) -> bool {
    err.is_retryable_conn()
        || matches!(
            err,
            Error::BodyTooLarge(_) | Error::Compression(_) | Error::Protocol { code: 0, .. }
        )
}

async fn read_body_limited(
    reader: &mut Reader,
    max_bytes: usize,
    budget: Option<&BodyBudget>,
) -> Result<Vec<u8>, Error> {
    let (code, text) = reader
        .read_status()
        .await
        .map_err(|err| err.timeout_context("BODY"))?;
    trace_response("BODY", code);
    if code != 222 {
        return Err(Error::from_status(code, text));
    }
    let mut reservation = budget.map(BodyBudget::response);
    let mut body = Vec::new();
    loop {
        let line = reader
            .read_line()
            .await
            .map_err(|err| err.timeout_context("BODY"))?;
        let Some(data) = unstuff(&line) else {
            if let Some(reservation) = &mut reservation {
                reservation.commit();
            }
            return Ok(body);
        };
        if body.len().saturating_add(data.len()).saturating_add(2) > max_bytes {
            return Err(Error::BodyTooLarge(max_bytes));
        }
        if let Some(reservation) = &mut reservation {
            reservation.reserve(data.len().saturating_add(2))?;
        }
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    }
}
