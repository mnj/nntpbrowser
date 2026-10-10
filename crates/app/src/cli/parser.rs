use std::path::PathBuf;

use engine::NewServer;

pub(super) const HELP: &str = "NNTPBrowser — native Usenet reader

No arguments launches the desktop window.

  help
  servers
  server-add --name NAME --host HOST [--port PORT] [--plain | --starttls]
             [--user USER] [--password PASSWORD] [--priority 1]
             [--connections 8] [--pipeline 8] [--fill] [--disabled]
  server-test ID
  server-delete ID
  groups SERVER_ID [--wildmat '*']
  subscribe SERVER_ID GROUP
  unsubscribe SERVER_ID GROUP
  sync GROUP_ID [--limit COUNT | --all]
  posts GROUP_ID [--search TEXT]
  open POST_ID
  queue POST_ID
  jobs
  pause JOB_ID
  resume JOB_ID
  cancel JOB_ID
  nzb PATH
  preview POST_ID

NNTP_PASSWORD supplies a server password when --password is omitted.
The desktop and CLI share the default data directory.
";

/// A parsed terminal command. Arguments exclude the executable name.
#[derive(Clone, PartialEq, Eq)]
pub enum CliCommand {
    Help,
    Servers,
    ServerAdd {
        server: NewServer,
        enabled: bool,
    },
    ServerTest(i64),
    ServerDelete(i64),
    Groups {
        server_id: i64,
        wildmat: String,
    },
    Subscribe {
        server_id: i64,
        group: String,
    },
    Unsubscribe {
        server_id: i64,
        group: String,
    },
    Sync {
        group_id: i64,
        limit: Option<Option<u64>>,
    },
    Posts {
        group_id: i64,
        search: String,
    },
    Open(i64),
    Queue(i64),
    Jobs,
    Pause(i64),
    Resume(i64),
    Cancel(i64),
    Nzb(PathBuf),
    Preview(i64),
}

/// Parse a command, returning `None` when the GUI should launch.
/// Errors never echo argument values, which may contain credentials.
pub fn parse_cli(args: &[String]) -> Result<Option<CliCommand>, String> {
    let Some(name) = args.first().map(String::as_str) else {
        return Ok(None);
    };
    let tail = &args[1..];
    let command = match name {
        "help" | "--help" | "-h" if tail.is_empty() => CliCommand::Help,
        "servers" if tail.is_empty() => CliCommand::Servers,
        "jobs" if tail.is_empty() => CliCommand::Jobs,
        "server-add" => parse_server(tail)?,
        "server-test" => CliCommand::ServerTest(one_id(tail)?),
        "server-delete" => CliCommand::ServerDelete(one_id(tail)?),
        "open" => CliCommand::Open(one_id(tail)?),
        "queue" => CliCommand::Queue(one_id(tail)?),
        "pause" => CliCommand::Pause(one_id(tail)?),
        "resume" => CliCommand::Resume(one_id(tail)?),
        "cancel" => CliCommand::Cancel(one_id(tail)?),
        "preview" => CliCommand::Preview(one_id(tail)?),
        "groups" => {
            let server_id = first_id(tail)?;
            let wildmat = optional_text(&tail[1..], "--wildmat", "*")?;
            CliCommand::Groups { server_id, wildmat }
        }
        "posts" => {
            let group_id = first_id(tail)?;
            let search = optional_text(&tail[1..], "--search", "")?;
            CliCommand::Posts { group_id, search }
        }
        "sync" => {
            let group_id = first_id(tail)?;
            let limit = match &tail[1..] {
                [] => None,
                [flag] if flag == "--all" => Some(None),
                [flag, count] if flag == "--limit" => {
                    let count = count
                        .parse::<u64>()
                        .ok()
                        .filter(|count| *count > 0)
                        .ok_or("--limit requires a positive article count")?;
                    Some(Some(count))
                }
                _ => {
                    return Err(
                        "sync requires GROUP_ID and optionally --limit COUNT or --all".into(),
                    )
                }
            };
            CliCommand::Sync { group_id, limit }
        }
        "subscribe" | "unsubscribe" if tail.len() == 2 => {
            let server_id = first_id(tail)?;
            let group = tail[1].clone();
            if group.is_empty() || group.chars().any(char::is_whitespace) || group.starts_with('-')
            {
                return Err("GROUP must be a nonempty newsgroup name without whitespace".into());
            }
            if name == "subscribe" {
                CliCommand::Subscribe { server_id, group }
            } else {
                CliCommand::Unsubscribe { server_id, group }
            }
        }
        "nzb" if tail.len() == 1 && !tail[0].is_empty() => CliCommand::Nzb(tail[0].clone().into()),
        _ => return Err("unknown command or invalid arguments; run nntpbrowser help".into()),
    };
    Ok(Some(command))
}

fn first_id(args: &[String]) -> Result<i64, String> {
    args.first()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|id| *id > 0)
        .ok_or_else(|| "a positive numeric ID is required".into())
}

fn one_id(args: &[String]) -> Result<i64, String> {
    if args.len() != 1 {
        return Err("exactly one numeric ID is required".into());
    }
    first_id(args)
}

fn optional_text(args: &[String], flag: &str, default: &str) -> Result<String, String> {
    match args {
        [] => Ok(default.into()),
        [name, value] if name == flag => Ok(value.clone()),
        _ => Err(format!("expected optionally {flag} VALUE")),
    }
}

fn parse_server(args: &[String]) -> Result<CliCommand, String> {
    let mut server = NewServer {
        name: String::new(),
        host: String::new(),
        port: 563,
        tls: true,
        starttls: false,
        username: None,
        password: None,
        priority: 1,
        max_connections: 8,
        pipeline_depth: 8,
        fill: false,
    };
    let mut enabled = true;
    let mut seen = std::collections::HashSet::new();
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        if !seen.insert(flag.as_str()) {
            return Err("duplicate server-add option".into());
        }
        match flag.as_str() {
            "--plain" => server.tls = false,
            "--starttls" => {
                server.tls = false;
                server.starttls = true;
            }
            "--fill" => server.fill = true,
            "--disabled" => enabled = false,
            "--name" | "--host" | "--port" | "--user" | "--password" | "--priority"
            | "--connections" | "--pipeline" => {
                let value = args
                    .next()
                    .filter(|value| !value.starts_with("--"))
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                match flag.as_str() {
                    "--name" => server.name = value.trim().into(),
                    "--host" => server.host = value.trim().into(),
                    "--user" => server.username = Some(value.clone()),
                    "--password" => server.password = Some(value.clone()),
                    "--port" => {
                        server.port = value
                            .parse()
                            .ok()
                            .filter(|port| *port > 0)
                            .ok_or("--port requires a port between 1 and 65535")?
                    }
                    "--priority" => {
                        server.priority = value
                            .parse()
                            .map_err(|_| "--priority requires an integer")?
                    }
                    "--connections" => {
                        server.max_connections = value
                            .parse()
                            .ok()
                            .filter(|n| *n > 0)
                            .ok_or("--connections requires a positive integer")?
                    }
                    "--pipeline" => {
                        server.pipeline_depth = value
                            .parse()
                            .ok()
                            .filter(|n| *n > 0)
                            .ok_or("--pipeline requires a positive integer")?
                    }
                    _ => unreachable!(),
                }
            }
            _ => return Err("unknown server-add option".into()),
        }
    }
    if server.name.is_empty() || server.host.is_empty() {
        return Err("server-add requires --name NAME and --host HOST".into());
    }
    if seen.contains("--plain") && seen.contains("--starttls") {
        return Err("--plain and --starttls cannot be used together".into());
    }
    if !server.tls && !seen.contains("--port") {
        server.port = 119;
    }
    Ok(CliCommand::ServerAdd { server, enabled })
}
