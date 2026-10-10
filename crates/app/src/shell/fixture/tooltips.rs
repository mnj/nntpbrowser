//! Deterministic traffic and connection tooltip scenes, without live sockets.
use super::*;

pub(super) fn seed(browser: &mut Browser, view: &str) {
    if view == "traffic" {
        browser.preview_speed_tooltip = true;
        browser.traffic.observe(
            (1, 1),
            engine::TrafficSnapshot {
                received: 96 * 1024 * 1024,
                sent: 384 * 1024,
                decoded_received: 288 * 1024 * 1024,
            },
            false,
        );
        browser.traffic.observe(
            (2, 2),
            engine::TrafficSnapshot {
                received: 32 * 1024 * 1024,
                sent: 128 * 1024,
                decoded_received: 64 * 1024 * 1024,
            },
            true,
        );
    }
    if view == "connections" {
        browser.preview_connections_tooltip = true;
        let receiving = engine::ConnectionStats {
            id: 101,
            authenticated: true,
            read_pending: true,
            write_pending: false,
            receiving: true,
            sending: false,
            received_bytes: 64 * 1024 * 1024,
            sent_bytes: 256 * 1024,
            received_per_second: 2_500_000.0,
            sent_per_second: 0.0,
        };
        let idle = engine::ConnectionStats {
            id: 104,
            authenticated: true,
            read_pending: false,
            write_pending: false,
            receiving: false,
            sending: false,
            received_bytes: 12 * 1024 * 1024,
            sent_bytes: 64 * 1024,
            received_per_second: 0.0,
            sent_per_second: 0.0,
        };
        browser.on_event(Event::ConnectionStats {
            server_id: 1,
            pool_id: 1,
            connections: vec![
                receiving,
                engine::ConnectionStats {
                    id: 102,
                    write_pending: true,
                    sending: true,
                    sent_per_second: 16_384.0,
                    ..idle.clone()
                },
                idle.clone(),
                engine::ConnectionStats {
                    id: 105,
                    authenticated: false,
                    read_pending: true,
                    ..idle.clone()
                },
                engine::ConnectionStats {
                    id: 103,
                    read_pending: true,
                    sending: true,
                    sent_per_second: 512.0,
                    ..idle.clone()
                },
            ],
        });
        browser.on_event(Event::ConnectionStats {
            server_id: 2,
            pool_id: 2,
            connections: vec![engine::ConnectionStats { id: 201, ..idle }],
        });
        browser.network_stats.insert((1, 1), (4, 4, 2_500_000.0));
        browser.network_stats.insert((2, 2), (0, 1, 0.0));
    }
}
