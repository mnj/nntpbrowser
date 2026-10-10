use std::time::{Duration, Instant};

use engine::{Command, Event, Handle, ServerRow};

use super::parser::HELP;
use super::CliCommand;

mod events;

/// Run a parsed command without opening an egui window.
pub fn run_cli(mut command: CliCommand) -> Result<(), String> {
    if command == CliCommand::Help {
        print!("{HELP}");
        return Ok(());
    }
    if let CliCommand::ServerAdd { server, .. } = &mut command {
        if server.password.is_none() {
            server.password = std::env::var("NNTP_PASSWORD").ok();
        }
    }
    let handle = Handle::start(crate::shell::default_data_dir());
    crate::shell::configure_handle(&handle);
    let mut terminal = Terminal {
        handle,
        passwords: Vec::new(),
    };
    if let CliCommand::ServerAdd { server, .. } = &command {
        terminal.remember_password(server.password.as_deref());
    }
    let servers = terminal.startup()?;
    terminal
        .execute(command, servers)
        .map_err(|error| terminal.scrub(&error))
}

struct Terminal {
    handle: Handle,
    passwords: Vec<String>,
}

impl Terminal {
    fn remember_password(&mut self, password: Option<&str>) {
        if let Some(password) = password.filter(|password| !password.is_empty()) {
            if !self.passwords.iter().any(|known| known == password) {
                self.passwords.push(password.into());
                self.passwords
                    .sort_by_key(|password| std::cmp::Reverse(password.len()));
            }
        }
    }

    fn scrub(&self, text: &str) -> String {
        self.passwords.iter().fold(text.into(), |text, password| {
            text.replace(password, "[redacted]")
        })
    }

    fn print(&self, text: impl AsRef<str>) {
        println!("{}", self.scrub(text.as_ref()));
    }

    fn startup(&mut self) -> Result<Vec<ServerRow>, String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut servers = None;
        let mut jobs_received = false;
        loop {
            let events = self.handle.poll();
            for event in events {
                match event {
                    Event::Servers(rows) => {
                        for server in &rows {
                            self.remember_password(server.password.as_deref());
                        }
                        servers = Some(rows);
                    }
                    Event::Notice(text) => return Err(self.scrub(&text)),
                    Event::Jobs(jobs) => {
                        for job in &jobs {
                            self.remember_password(job.password.as_deref());
                        }
                        jobs_received = true;
                    }
                    _ => {}
                }
            }
            if jobs_received {
                if let Some(servers) = servers.take() {
                    return Ok(servers);
                }
            }
            if Instant::now() >= deadline {
                return Err("timed out starting the engine".into());
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    fn execute(&mut self, command: CliCommand, servers: Vec<ServerRow>) -> Result<(), String> {
        let server_id = match &command {
            CliCommand::ServerTest(id) | CliCommand::ServerDelete(id) => Some(*id),
            CliCommand::Groups { server_id, .. }
            | CliCommand::Subscribe { server_id, .. }
            | CliCommand::Unsubscribe { server_id, .. } => Some(*server_id),
            _ => None,
        };
        if server_id.is_some_and(|id| !servers.iter().any(|server| server.id == id)) {
            return Err("server ID was not found".into());
        }
        let engine_command = match &command {
            CliCommand::Help => unreachable!(),
            CliCommand::Servers => {
                self.print_servers(&servers);
                return Ok(());
            }
            CliCommand::ServerAdd { server, enabled } => Command::SaveServerEnabled {
                server: server.clone(),
                enabled: *enabled,
            },
            CliCommand::ServerTest(id) => Command::TestServer(*id),
            CliCommand::ServerDelete(id) => Command::DeleteServer(*id),
            CliCommand::Groups { server_id, wildmat } => Command::ListActive {
                server_id: *server_id,
                wildmat: wildmat.clone(),
            },
            CliCommand::Subscribe { server_id, group } => Command::Subscribe {
                server_id: *server_id,
                name: group.clone(),
            },
            CliCommand::Unsubscribe { server_id, group } => Command::Unsubscribe {
                server_id: *server_id,
                name: group.clone(),
            },
            CliCommand::Sync {
                group_id,
                limit: None,
            } => Command::SyncGroup(*group_id),
            CliCommand::Sync {
                group_id,
                limit: Some(limit),
            } => Command::FetchHeaders {
                group_id: *group_id,
                limit: *limit,
            },
            CliCommand::Posts { group_id, search } => Command::LoadPosts {
                group_id: *group_id,
                after: None,
                search: search.clone(),
            },
            CliCommand::Open(id) => Command::OpenPost(*id),
            CliCommand::Queue(id) => Command::QueuePost { post_id: *id },
            CliCommand::Jobs => Command::Refresh,
            CliCommand::Pause(id) => Command::Pause(*id),
            CliCommand::Resume(id) => Command::Resume(*id),
            CliCommand::Cancel(id) => Command::Cancel(*id),
            CliCommand::Nzb(path) => {
                let xml = std::fs::read_to_string(path)
                    .map_err(|err| format!("cannot read NZB: {err}"))?;
                let name = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("nzb");
                let name: String = name
                    .chars()
                    .map(|ch| {
                        if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
                            ch
                        } else {
                            '_'
                        }
                    })
                    .collect();
                Command::ImportNzb {
                    xml,
                    dest: crate::shell::downloads_dir(&self.handle).join(name),
                }
            }
            CliCommand::Preview(id) => Command::Preview(*id),
        };
        self.handle.send(engine_command);
        self.wait(command, &servers)
    }

    fn print_servers(&self, servers: &[ServerRow]) {
        self.print("ID\tNAME\tHOST\tPORT\tSECURITY\tENABLED\tFILL\tPRIORITY\tHEADERS");
        for server in servers {
            self.print(format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                server.id,
                server.name,
                server.host,
                server.port,
                if server.starttls {
                    "STARTTLS"
                } else if server.tls {
                    "implicit TLS"
                } else {
                    "plain"
                },
                server.enabled,
                server.fill,
                server.priority,
                server.header_compression
            ));
        }
    }
}
