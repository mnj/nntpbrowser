//! Sample pools independently so header commands never stall live status updates.
use super::Event;
use crate::TransferRate;
use nntp::Pool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
mod connections;

pub(super) fn watch_pool(pool: &Arc<Pool>, events: UnboundedSender<Event>) {
    let server_id = pool.server_id();
    let pool_id = pool.metrics_id();
    let weak = Arc::downgrade(pool);
    let traffic = pool.traffic_counter();
    let started = Instant::now();
    let mut rate = TransferRate::new(pool.received_bytes(), started);
    let mut connections = connections::Tracker::new(started);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let Some(pool) = weak.upgrade() else {
                let _ = events.send(Event::NetworkTraffic {
                    server_id,
                    pool_id,
                    traffic: traffic.snapshot(),
                    closed: true,
                });
                let _ = events.send(Event::ConnectionStats {
                    server_id,
                    pool_id,
                    connections: Vec::new(),
                });
                let _ = events.send(Event::NetworkStats {
                    server_id,
                    pool_id,
                    open: 0,
                    active: 0,
                    bytes_per_second: 0.0,
                });
                return;
            };
            let now = Instant::now();
            if events
                .send(Event::ConnectionStats {
                    server_id,
                    pool_id,
                    connections: connections.sample(pool.connection_stats(), now),
                })
                .is_err()
            {
                return;
            }
            if events
                .send(Event::NetworkTraffic {
                    server_id,
                    pool_id,
                    traffic: pool.traffic(),
                    closed: false,
                })
                .is_err()
            {
                return;
            }
            rate.observe(pool.received_bytes(), now);
            let speed = rate.bytes_per_second(now);
            if events
                .send(Event::NetworkStats {
                    server_id,
                    pool_id,
                    open: pool.open_connections(),
                    active: pool.take_active_peak(),
                    bytes_per_second: speed,
                })
                .is_err()
            {
                return;
            }
        }
    });
}
