//! Per-server connection pool and a priority/fill farm.
//!
//! Header and BODY operations share the configured semaphore. BODY sockets are
//! never compressed. Only [`Error::Missing`] moves a body to the next server.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, Semaphore};

use crate::conn::Connection;
use crate::error::Error;
use crate::types::{
    ArticleRef, GroupInfo, GroupStatus, HeaderStrategy, OverviewLine, ServerEndpoint,
};
use crate::{BodyBudget, Traffic, TrafficSnapshot};

pub(crate) mod metrics;

pub struct Pool {
    metrics_id: u64,
    traffic: Arc<Traffic>,
    metrics: Arc<metrics::Metrics>,
    connections: Arc<crate::connection_stats::Registry>,
    endpoint: ServerEndpoint,
    header: Mutex<Option<Connection>>,
    idle: Mutex<Vec<Connection>>,
    slots: Arc<Semaphore>,
    strategy: Mutex<HeaderStrategy>,
    selected_group: Mutex<Option<String>>,
}

impl Pool {
    pub fn new(endpoint: ServerEndpoint) -> Self {
        static NEXT_METRICS_ID: AtomicU64 = AtomicU64::new(1);
        let slots = endpoint.max_connections.clamp(1, Semaphore::MAX_PERMITS);
        Self {
            metrics_id: NEXT_METRICS_ID.fetch_add(1, Ordering::Relaxed),
            traffic: Arc::new(Traffic::default()),
            metrics: Arc::new(metrics::Metrics::default()),
            connections: Arc::new(crate::connection_stats::Registry::default()),
            endpoint,
            header: Mutex::new(None),
            idle: Mutex::new(Vec::new()),
            slots: Arc::new(Semaphore::new(slots)),
            strategy: Mutex::new(HeaderStrategy::Unknown),
            selected_group: Mutex::new(None),
        }
    }

    /// Effective shared ceiling for BODY and header operations on this server.
    pub fn max_connections(&self) -> usize {
        self.endpoint
            .max_connections
            .clamp(1, Semaphore::MAX_PERMITS)
    }

    pub fn server_id(&self) -> i64 {
        self.endpoint.id
    }

    /// Requests using a connection, including connection establishment; idle
    /// reusable sockets do not count as active.
    pub fn active_connections(&self) -> usize {
        self.metrics.active()
    }

    /// Authenticated sockets owned by this pool, including idle reusable ones.
    pub fn open_connections(&self) -> usize {
        self.metrics.open()
    }

    /// Highest number of busy connections since the previous call. Sampling
    /// this preserves short transfers which finish between status updates.
    pub fn take_active_peak(&self) -> usize {
        self.metrics.take_peak()
    }

    async fn acquire_slot(&self) -> Result<metrics::Lease<'_>, Error> {
        let permit = self.slots.acquire().await.map_err(|_| Error::Closed)?;
        Ok(self.metrics.lease(permit))
    }

    pub fn metrics_id(&self) -> u64 {
        self.metrics_id
    }

    /// NNTP transport bytes received before decompression, excluding TLS framing.
    pub fn received_bytes(&self) -> u64 {
        self.traffic().received
    }

    /// Cumulative traffic across every connection, including setup and retries.
    pub fn traffic(&self) -> TrafficSnapshot {
        self.traffic.snapshot()
    }

    /// Retain final totals after this pool is retired, without keeping sockets.
    pub fn traffic_counter(&self) -> Arc<Traffic> {
        self.traffic.clone()
    }

    /// Live setup, authenticated and idle connection states, sorted by stable
    /// per-pool ID. Snapshots contain no command arguments or response contents.
    pub fn connection_stats(&self) -> Vec<crate::ConnectionSnapshot> {
        self.connections.snapshot()
    }

    pub async fn header_strategy(&self) -> HeaderStrategy {
        *self.strategy.lock().await
    }

    pub async fn set_header_strategy(&self, strategy: HeaderStrategy) {
        *self.strategy.lock().await = strategy;
    }

    pub async fn fetch_overview<F>(
        &self,
        first: u64,
        last: u64,
        mut on_line: F,
    ) -> Result<HeaderStrategy, Error>
    where
        F: FnMut(OverviewLine) + Send,
    {
        let _permit = self.acquire_slot().await?;
        let hint = *self.strategy.lock().await;
        let mut conn = self.take_overview_header().await?;
        let mut emitted = false;
        let result = conn
            .fetch_overview(first, last, hint, |line| {
                emitted = true;
                on_line(line);
            })
            .await;
        match result {
            Ok(strategy) => {
                *self.strategy.lock().await = strategy;
                *self.header.lock().await = Some(conn);
                Ok(strategy)
            }
            Err(err)
                if !emitted
                    && (err.is_retryable_conn()
                        || matches!(err, Error::Compression(_) | Error::Unsupported)) =>
            {
                drop(conn);
                let mut conn = self.connect_new().await?;
                self.restore_group(&mut conn).await?;
                // A speculative extension must not keep a usable group from
                // downloading. Start with a fresh, synchronized plain session.
                let strategy = conn
                    .fetch_overview(first, last, HeaderStrategy::Plain, on_line)
                    .await?;
                *self.strategy.lock().await = strategy;
                *self.header.lock().await = Some(conn);
                Ok(strategy)
            }
            Err(err) => {
                if reusable(&err) {
                    *self.header.lock().await = Some(conn);
                }
                Err(err)
            }
        }
    }

    pub async fn list_active(&self, wildmat: &str) -> Result<Vec<GroupInfo>, Error> {
        let mut groups = Vec::new();
        self.list_active_each(wildmat, |group| groups.push(group))
            .await?;
        Ok(groups)
    }

    /// Retry only before the first callback, avoiding duplicate streamed groups.
    pub async fn list_active_each<F>(&self, wildmat: &str, mut on_group: F) -> Result<(), Error>
    where
        F: FnMut(GroupInfo) + Send,
    {
        let _permit = self.acquire_slot().await?;
        let mut conn = self.take_header().await?;
        let mut emitted = false;
        let result = conn
            .list_active_each(wildmat, |group| {
                emitted = true;
                on_group(group);
            })
            .await;
        match result {
            Ok(()) => {
                *self.header.lock().await = Some(conn);
                Ok(())
            }
            Err(err) if err.is_retryable_conn() && !emitted => {
                drop(conn);
                let mut conn = self.connect_new().await?;
                conn.list_active_each(wildmat, on_group).await?;
                *self.header.lock().await = Some(conn);
                Ok(())
            }
            Err(err) => {
                if reusable(&err) {
                    *self.header.lock().await = Some(conn);
                }
                Err(err)
            }
        }
    }

    pub async fn group(&self, name: &str) -> Result<GroupStatus, Error> {
        let _permit = self.acquire_slot().await?;
        let mut conn = self.take_header().await?;
        match conn.group(name).await {
            Ok(status) => {
                *self.selected_group.lock().await = Some(name.to_owned());
                *self.header.lock().await = Some(conn);
                Ok(status)
            }
            Err(err) if err.is_retryable_conn() => {
                drop(conn);
                let mut conn = self.connect_new().await?;
                let status = conn.group(name).await?;
                *self.selected_group.lock().await = Some(name.to_owned());
                *self.header.lock().await = Some(conn);
                Ok(status)
            }
            Err(err) => {
                if reusable(&err) {
                    *self.header.lock().await = Some(conn);
                }
                Err(err)
            }
        }
    }

    pub async fn body(&self, id: &ArticleRef) -> Result<Vec<u8>, Error> {
        self.body_limited(id, usize::MAX).await
    }

    pub async fn body_limited(&self, id: &ArticleRef, max_bytes: usize) -> Result<Vec<u8>, Error> {
        let _permit = self.acquire_slot().await?;
        let mut conn = self.pop_or_connect().await?;
        match conn.body_limited(id, max_bytes).await {
            Ok(bytes) => {
                self.idle.lock().await.push(conn);
                Ok(bytes)
            }
            Err(err) if err.is_retryable_conn() => {
                drop(conn);
                let mut conn = self.connect_new().await?;
                match conn.body_limited(id, max_bytes).await {
                    Ok(bytes) => {
                        self.idle.lock().await.push(conn);
                        Ok(bytes)
                    }
                    Err(err) => {
                        self.keep_download(conn, &err).await;
                        Err(err)
                    }
                }
            }
            Err(err) => {
                self.keep_download(conn, &err).await;
                Err(err)
            }
        }
    }

    pub async fn body_many(
        &self,
        ids: &[ArticleRef],
    ) -> Result<Vec<Result<Vec<u8>, Error>>, Error> {
        self.body_many_limited(ids, &vec![usize::MAX; ids.len()])
            .await
    }

    pub async fn body_many_limited(
        &self,
        ids: &[ArticleRef],
        limits: &[usize],
    ) -> Result<Vec<Result<Vec<u8>, Error>>, Error> {
        self.body_many_with_budget(ids, limits, None).await
    }

    /// Use one shared budget for all parallel BODY windows of a bounded operation.
    /// Only failed response prefixes are refunded; successful bytes stay charged.
    pub async fn body_many_budgeted(
        &self,
        ids: &[ArticleRef],
        limits: &[usize],
        budget: &BodyBudget,
    ) -> Result<Vec<Result<Vec<u8>, Error>>, Error> {
        self.body_many_with_budget(ids, limits, Some(budget)).await
    }

    async fn body_many_with_budget(
        &self,
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
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let _permit = self.acquire_slot().await?;
        let depth = self.pipeline_depth();
        let mut connection = Some(self.pop_or_connect().await?);
        let mut output = Vec::with_capacity(ids.len());
        let mut index = 0;
        while index < ids.len() {
            if connection.is_none() {
                connection = match self.pop_or_connect().await {
                    Ok(conn) => Some(conn),
                    Err(error) => return retain_completed(output, error, ids.len() - index),
                };
            }
            let end = index.saturating_add(depth).min(ids.len());
            for attempt in 0..2 {
                let result = connection
                    .as_mut()
                    .expect("download connection")
                    .body_pipeline_with_budget(&ids[index..end], &limits[index..end], budget)
                    .await;
                let untouched_retry = match &result {
                    Ok(chunk) => {
                        !chunk.is_empty()
                            && chunk
                                .iter()
                                .all(|reply| reply.as_ref().is_err_and(Error::is_retryable_conn))
                    }
                    Err(error) => error.is_retryable_conn(),
                };
                if untouched_retry && attempt == 0 {
                    drop(connection.take());
                    connection = match self.connect_new().await {
                        Ok(conn) => Some(conn),
                        Err(error) => return retain_completed(output, error, ids.len() - index),
                    };
                    continue;
                }
                match result {
                    Ok(chunk) => {
                        let broken = chunk
                            .iter()
                            .any(|reply| reply.as_ref().is_err_and(|error| !reusable(error)));
                        output.extend(chunk);
                        index = end;
                        if broken {
                            drop(connection.take());
                        }
                    }
                    Err(error) => {
                        if let Some(conn) = connection.take() {
                            self.keep_download(conn, &error).await;
                        }
                        return retain_completed(output, error, ids.len() - index);
                    }
                }
                break;
            }
        }
        if let Some(conn) = connection {
            self.idle.lock().await.push(conn);
        }
        Ok(output)
    }

    async fn take_header(&self) -> Result<Connection, Error> {
        if let Some(conn) = self.header.lock().await.take() {
            return Ok(conn);
        }
        self.pop_or_connect().await
    }

    async fn take_overview_header(&self) -> Result<Connection, Error> {
        if let Some(conn) = self.header.lock().await.take() {
            return Ok(conn);
        }
        let mut conn = self.pop_or_connect().await?;
        self.restore_group(&mut conn).await?;
        Ok(conn)
    }

    async fn restore_group(&self, conn: &mut Connection) -> Result<(), Error> {
        if let Some(name) = self.selected_group.lock().await.clone() {
            conn.group(&name).await?;
        }
        Ok(())
    }

    async fn pop_or_connect(&self) -> Result<Connection, Error> {
        // Compressed header sockets cannot serve BODY. Close the idle header
        // before opening a download socket, including on one-slot servers.
        self.header.lock().await.take();
        if let Some(conn) = self.idle.lock().await.pop() {
            return Ok(conn);
        }
        self.connect_new().await
    }

    pub fn pipeline_depth(&self) -> usize {
        self.endpoint.pipeline_depth.clamp(1, 256)
    }

    async fn connect_new(&self) -> Result<Connection, Error> {
        let mut last = Error::Connect("no attempt".into());
        for attempt in 0..3 {
            let registration = self.connections.register(self.traffic.clone());
            match Connection::connect_tracked(&self.endpoint.config, registration).await {
                Ok(mut conn) => {
                    conn.measure_socket(self.metrics.clone());
                    return Ok(conn);
                }
                Err(err) => {
                    last = err;
                    if attempt + 1 < 3 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
            }
        }
        Err(last)
    }

    async fn keep_download(&self, conn: Connection, err: &Error) {
        if reusable(err) {
            self.idle.lock().await.push(conn);
        }
    }
}

/// A late connection failure must not erase successful earlier windows.
fn retain_completed(
    mut output: Vec<Result<Vec<u8>, Error>>,
    error: Error,
    remaining: usize,
) -> Result<Vec<Result<Vec<u8>, Error>>, Error> {
    if output.is_empty() {
        return Err(error);
    }
    let message = error.to_string();
    if remaining > 0 {
        output.push(Err(error));
    }
    output.extend((1..remaining).map(|_| Err(Error::Connect(message.clone()))));
    Ok(output)
}

fn reusable(err: &Error) -> bool {
    !err.is_retryable_conn()
        && !matches!(
            err,
            Error::Compression(_)
                | Error::BodyTooLarge(_)
                | Error::Protocol {
                    code: 0 | 101 | 215 | 220 | 221 | 222 | 224 | 230 | 231,
                    ..
                }
        )
}

/// Tries non-fill servers first, lowest priority number first.
/// Fill servers run only after every non-fill server returned [`Error::Missing`].
pub struct Farm {
    pools: Vec<Pool>,
}

impl Farm {
    pub fn new(endpoints: Vec<ServerEndpoint>) -> Self {
        Self {
            pools: endpoints.into_iter().map(Pool::new).collect(),
        }
    }

    pub fn pools(&self) -> &[Pool] {
        &self.pools
    }

    pub async fn body(&self, id: &ArticleRef) -> Result<(i64, Vec<u8>), Error> {
        if self.pools.is_empty() {
            return Err(Error::Connect("no servers configured".into()));
        }
        let mut order: Vec<usize> = (0..self.pools.len()).collect();
        order.sort_by_key(|index| {
            let pool = &self.pools[*index];
            (pool.endpoint.fill, pool.endpoint.priority)
        });
        let mut last_missing = Error::Missing;
        for index in order {
            let pool = &self.pools[index];
            match pool.body(id).await {
                Ok(bytes) => return Ok((pool.server_id(), bytes)),
                Err(Error::Missing) => last_missing = Error::Missing,
                Err(err) => return Err(err),
            }
        }
        Err(last_missing)
    }
}
