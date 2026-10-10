//! Connection configuration and streamed server group discovery.

use super::{Event, ListedGroup, Worker};
use nntp::{Config, HeaderStrategy, Pool, ServerEndpoint, TlsMode};
use std::time::Duration;
use store::ServerRow;

impl Worker {
    pub(super) fn delete_server(&mut self, id: i64) {
        if self.server(id).is_none() {
            self.notice("delete server: server was not found".into());
            return;
        }
        // SQLite can reuse this ID. Remove its old provider's article cache
        // before deleting the row so a reused ID starts without stale ranges.
        if let Err(err) = self.headers.remove_server(id) {
            self.notice(format!("delete server cache: {err}"));
            return;
        }
        self.pools.remove(&id);
        if let Err(err) = self.catalog.delete_server(id) {
            self.notice(format!("delete server: {err}"));
        }
    }

    pub(super) async fn test_server(&mut self, id: i64) {
        let Some(server) = self.server(id) else {
            self.notice("server is gone".into());
            return;
        };
        self.ensure_pool(&server).await;
        let mut count = 0u64;
        match self
            .pools
            .get(&id)
            .unwrap()
            .list_active_each("*", |_| count += 1)
            .await
        {
            Ok(()) => self.notice(format!("connected, {count} group(s)")),
            Err(err) => self.notice(format!("connection failed: {err}")),
        }
    }

    pub(super) async fn list_active(&mut self, server_id: i64, wildmat: &str) {
        let _ = self.evt.send(Event::GroupSample {
            server_id,
            groups: Vec::new(),
            received: 0,
            done: false,
        });
        let Some(server) = self.server(server_id) else {
            self.notice("group list: server is gone".into());
            return;
        };
        self.ensure_pool(&server).await;
        let mut batch = Vec::with_capacity(450);
        let mut received = 0u64;
        let evt = self.evt.clone();
        let result = self
            .pools
            .get(&server_id)
            .unwrap()
            .list_active_each(wildmat, |group| {
                received += 1;
                batch.push(ListedGroup {
                    name: group.name,
                    low: group.low,
                    high: group.high,
                    status: group.status,
                });
                if batch.len() >= 450 {
                    let _ = evt.send(Event::GroupSample {
                        server_id,
                        groups: std::mem::replace(&mut batch, Vec::with_capacity(450)),
                        received,
                        done: false,
                    });
                }
            })
            .await;
        // Deliver even a short final batch after a partial network failure.
        let _ = self.evt.send(Event::GroupSample {
            server_id,
            groups: batch,
            received,
            done: result.is_ok(),
        });
        if let Err(err) = result {
            self.notice(format!("group list: {err}"));
        }
    }

    pub(super) async fn ensure_pool(&mut self, server: &ServerRow) {
        if self.pools.contains_key(&server.id) {
            return;
        }
        let pool = std::sync::Arc::new(Pool::new(endpoint(server, self.network_timeout_secs)));
        pool.set_header_strategy(parse_strategy(&server.header_compression))
            .await;
        super::telemetry::watch_pool(&pool, self.evt.clone());
        self.pools.insert(server.id, pool);
    }
}

fn endpoint(server: &ServerRow, timeout_secs: u64) -> ServerEndpoint {
    ServerEndpoint {
        id: server.id,
        config: Config {
            host: server.host.clone(),
            port: server.port,
            tls: if server.starttls {
                TlsMode::StartTls
            } else if server.tls {
                TlsMode::Implicit
            } else {
                TlsMode::Plain
            },
            username: server.username.clone(),
            password: server.password.clone(),
            timeout: Duration::from_secs(timeout_secs),
        },
        priority: server.priority,
        fill: server.fill,
        max_connections: server.max_connections.clamp(1, 128) as usize,
        pipeline_depth: server.pipeline_depth.clamp(1, 256) as usize,
    }
}

fn parse_strategy(text: &str) -> HeaderStrategy {
    match text {
        "xzver" => HeaderStrategy::Xzver,
        "compress-deflate" => HeaderStrategy::CompressDeflate,
        "xfeature-gzip" => HeaderStrategy::XfeatureGzip,
        "plain" => HeaderStrategy::Plain,
        _ => HeaderStrategy::Unknown,
    }
}
