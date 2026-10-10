//! Scrollable connection details for the fixed-width footer segment.
use super::*;
use eframe::egui::RichText;

impl Browser {
    pub(super) fn show_connections_tooltip(&self, ui: &mut egui::Ui) {
        ui.set_width(440.0);
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
        ui.label(
            RichText::new("Live connections")
                .strong()
                .color(theme::TEXT),
        );
        ui.label(
            RichText::new("Activity: last 250 ms · Speeds: 5-second average")
                .small()
                .color(theme::DIM),
        );
        ui.label(
            RichText::new("Open sockets include idle connections; new ones open on demand.")
                .small()
                .color(theme::DIM),
        );
        ui.separator();
        let mut rows: Vec<_> = self
            .connections
            .iter()
            .flat_map(|(&(server, pool), connections)| {
                connections
                    .iter()
                    .map(move |connection| (server, pool, connection))
            })
            .collect();
        rows.sort_by_key(|(server, pool, connection)| (*server, *pool, connection.id));
        if rows.is_empty() {
            ui.label("No connections open.");
            return;
        }
        egui::ScrollArea::vertical()
            .max_height(340.0)
            .show(ui, |ui| {
                for (index, &(server_id, pool_id, connection)) in rows.iter().enumerate() {
                    let name = self
                        .servers
                        .iter()
                        .find(|server| server.id == server_id)
                        .map(|server| self.scrub_text(&server.name))
                        .unwrap_or_else(|| format!("Server {server_id}"));
                    ui.push_id((server_id, pool_id, connection.id), |ui| {
                        ui.label(
                            RichText::new(format!("{name} · Connection {}", index + 1))
                                .strong()
                                .color(theme::TEXT),
                        );
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(state(connection)).color(
                                if connection.read_pending
                                    || connection.write_pending
                                    || connection.receiving
                                    || connection.sending
                                {
                                    theme::ACCENT
                                } else {
                                    theme::DIM
                                },
                            ));
                            ui.label(format!(
                                "↓ {}/s   ↑ {}/s",
                                bytes(connection.received_per_second.max(0.0) as u64),
                                bytes(connection.sent_per_second.max(0.0) as u64)
                            ));
                        });
                        ui.label(
                            RichText::new(format!(
                                "Received {} · Sent {}",
                                bytes(connection.received_bytes),
                                bytes(connection.sent_bytes)
                            ))
                            .small()
                            .color(theme::DIM),
                        );
                        ui.separator();
                    });
                }
            });
        ui.label(
            RichText::new("Network bytes exclude TLS overhead.")
                .small()
                .color(theme::DIM),
        );
    }
}

fn bytes(value: u64) -> String {
    format_bytes(i64::try_from(value).unwrap_or(i64::MAX))
}

fn state(connection: &engine::ConnectionStats) -> &'static str {
    if !connection.authenticated {
        return if connection.read_pending {
            "Connecting · waiting for server"
        } else if connection.write_pending {
            "Connecting · sending handshake"
        } else {
            "Connecting"
        };
    }
    if connection.read_pending {
        return if connection.receiving {
            "Receiving"
        } else {
            "Waiting for data"
        };
    }
    if connection.write_pending {
        return if connection.sending {
            "Sending"
        } else {
            "Waiting to send"
        };
    }
    match (connection.receiving, connection.sending) {
        (true, true) => "Sending and receiving",
        (true, false) => "Receiving",
        (false, true) => "Sending",
        (false, false) => "Idle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_one_pool_removes_only_its_connection_rows() {
        let mut browser = super::super::tests::browser();
        let connection = engine::ConnectionStats {
            id: 1,
            authenticated: true,
            read_pending: false,
            write_pending: false,
            receiving: false,
            sending: false,
            received_bytes: 100,
            sent_bytes: 20,
            received_per_second: 0.0,
            sent_per_second: 0.0,
        };
        for pool_id in [1, 2] {
            browser.on_event(Event::ConnectionStats {
                server_id: 1,
                pool_id,
                connections: vec![connection.clone()],
            });
        }
        browser.on_event(Event::ConnectionStats {
            server_id: 1,
            pool_id: 1,
            connections: vec![],
        });
        assert!(!browser.connections.contains_key(&(1, 1)));
        assert_eq!(browser.connections[&(1, 2)], vec![connection]);
        browser.on_event(Event::ConnectionStats {
            server_id: 1,
            pool_id: 2,
            connections: vec![],
        });
        assert!(browser.connections.is_empty());
    }

    #[test]
    fn pending_response_is_waiting_even_after_sending_a_command() {
        let mut connection = engine::ConnectionStats {
            id: 1,
            authenticated: true,
            read_pending: true,
            write_pending: false,
            receiving: false,
            sending: true,
            received_bytes: 100,
            sent_bytes: 20,
            received_per_second: 20.0,
            sent_per_second: 4.0,
        };
        assert_eq!(state(&connection), "Waiting for data");
        connection.receiving = true;
        assert_eq!(state(&connection), "Receiving");
        connection.authenticated = false;
        assert_eq!(state(&connection), "Connecting · waiting for server");
        connection.read_pending = false;
        connection.write_pending = true;
        assert_eq!(state(&connection), "Connecting · sending handshake");
        connection.authenticated = true;
        assert_eq!(state(&connection), "Sending");
        connection.sending = false;
        assert_eq!(state(&connection), "Waiting to send");
        connection.write_pending = false;
        connection.receiving = false;
        assert_eq!(state(&connection), "Idle");
    }
}
