use std::time::{Duration, Instant};

use engine::{Command, Event, ServerRow};

use super::{CliCommand, Terminal};

impl Terminal {
    pub(super) fn wait(
        &mut self,
        command: CliCommand,
        previous_servers: &[ServerRow],
    ) -> Result<(), String> {
        let timeout = if matches!(command, CliCommand::Groups { .. } | CliCommand::Sync { .. }) {
            180
        } else {
            120
        };
        let deadline = Instant::now() + Duration::from_secs(timeout);
        let mut last_post_key = None;
        let mut header_progress = None;
        if matches!(command, CliCommand::Posts { .. }) {
            self.print("ID\tDATE_UNIX\tBYTES\tFILES\tPARTS\tTITLE\tPOSTER");
        }
        if matches!(command, CliCommand::Groups { .. }) {
            self.print("NAME\tARTICLES\tLOW\tHIGH\tSTATUS");
        }
        loop {
            let mut complete = false;
            for event in self.handle.poll() {
                let event = match event {
                    Event::GroupRows { groups, .. } => Event::Groups(groups),
                    Event::PostsPage {
                        group_id,
                        search,
                        after,
                        posts,
                    } => {
                        if after == last_post_key
                            && matches!(&command, CliCommand::Posts { group_id: requested, search: requested_search } if *requested == group_id && *requested_search == search)
                        {
                            Event::Posts(posts)
                        } else {
                            continue;
                        }
                    }
                    event => event,
                };
                match event {
                    Event::Notice(text) => {
                        let success = match command {
                            CliCommand::ServerTest(_) => text.starts_with("connected, overview "),
                            CliCommand::Sync { .. } => {
                                text.starts_with("synced ")
                                    || text.contains(" is up to date")
                                    || text.contains(" has no articles in ")
                                    || text.ends_with(" is empty")
                            }
                            CliCommand::Queue(_) => text.starts_with("queued "),
                            CliCommand::Nzb(_) => text.starts_with("imported NZB, "),
                            _ => false,
                        };
                        if !success {
                            return Err(self.scrub(&text));
                        }
                        self.print(&text);
                        // Queue/import finish at their following Jobs snapshot.
                        if matches!(command, CliCommand::ServerTest(_) | CliCommand::Sync { .. }) {
                            complete = true;
                        }
                    }
                    Event::Servers(rows) => match &command {
                        CliCommand::ServerAdd { enabled, .. } => {
                            let id = rows
                                .iter()
                                .map(|row| row.id)
                                .max()
                                .ok_or("saved server was not found")?;
                            if previous_servers.iter().any(|row| row.id == id) {
                                return Err("server-add did not create a new server".into());
                            }
                            if rows
                                .iter()
                                .find(|row| row.id == id)
                                .is_some_and(|row| row.enabled != *enabled)
                            {
                                return Err("server's enabled state was not saved".into());
                            }
                            self.print(format!("saved server {id}"));
                            complete = true;
                        }
                        CliCommand::ServerDelete(id) if !rows.iter().any(|row| row.id == *id) => {
                            self.print(format!("deleted server {id}"));
                            complete = true;
                        }
                        _ => {}
                    },
                    Event::Groups(rows) => match &command {
                        CliCommand::Subscribe { server_id, group }
                            if rows.iter().any(|row| {
                                row.server_id == *server_id && row.name == *group && row.subscribed
                            }) =>
                        {
                            let id = rows
                                .iter()
                                .find(|row| row.server_id == *server_id && row.name == *group)
                                .map(|row| row.id)
                                .expect("matched subscribed group");
                            self.print(format!("subscribed to {group} (group ID {id})"));
                            complete = true;
                        }
                        CliCommand::Unsubscribe { server_id, group }
                            if !rows.iter().any(|row| {
                                row.server_id == *server_id && row.name == *group && row.subscribed
                            }) =>
                        {
                            self.print(format!("unsubscribed from {group}"));
                            complete = true;
                        }
                        _ => {}
                    },
                    Event::GroupSample {
                        server_id,
                        groups,
                        received,
                        done,
                    } => {
                        if matches!(&command, CliCommand::Groups { server_id: requested, .. } if *requested == server_id)
                        {
                            for group in groups {
                                self.print(format!(
                                    "{}\t{}\t{}\t{}\t{}",
                                    group.name,
                                    group.posts(),
                                    group.low,
                                    group.high,
                                    group.status
                                ));
                            }
                            eprintln!(
                                "group list: {received} groups{}",
                                if done { " (complete)" } else { "" }
                            );
                            complete = done;
                        }
                    }
                    Event::Posts(rows) => {
                        if let CliCommand::Posts { group_id, search } = &command {
                            for post in &rows {
                                self.print(format!(
                                    "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                                    post.id,
                                    post.posted_at,
                                    post.total_bytes,
                                    post.file_count,
                                    post.segment_count,
                                    post.title,
                                    post.poster
                                ));
                            }
                            let after = rows.last().map(|row| (row.posted_at, row.id));
                            if rows.len() < 80 {
                                complete = true;
                            } else if after == last_post_key {
                                return Err("post pagination stopped advancing".into());
                            } else {
                                last_post_key = after;
                                self.handle.send(Command::LoadPosts {
                                    group_id: *group_id,
                                    after,
                                    search: search.clone(),
                                });
                            }
                        }
                    }
                    Event::Files { post_id, files } => {
                        if matches!(command, CliCommand::Open(id) if id == post_id) {
                            self.print("ID\tKIND\tBYTES\tPARTS\tNAME");
                            for file in files {
                                self.print(format!(
                                    "{}\t{}\t{}\t{}\t{}",
                                    file.id,
                                    file.kind.as_str(),
                                    file.size,
                                    file.expected_segments,
                                    file.name
                                ));
                            }
                            complete = true;
                        }
                    }
                    Event::Jobs(jobs) => {
                        let accepted = match command {
                            CliCommand::Jobs | CliCommand::Nzb(_) => true,
                            CliCommand::Queue(post_id) => {
                                jobs.iter().any(|job| job.post_id == Some(post_id))
                            }
                            CliCommand::Pause(id)
                            | CliCommand::Resume(id)
                            | CliCommand::Cancel(id) => {
                                let job = jobs
                                    .iter()
                                    .find(|job| job.id == id)
                                    .ok_or("job ID was not found")?;
                                let expected = match command {
                                    CliCommand::Pause(_) => "paused",
                                    CliCommand::Cancel(_) => "cancelled",
                                    _ => "queued",
                                };
                                if job.state != expected {
                                    return Err(format!(
                                        "job is {}, expected {expected}",
                                        job.state
                                    ));
                                }
                                true
                            }
                            _ => false,
                        };
                        if accepted {
                            for job in &jobs {
                                self.remember_password(job.password.as_deref());
                            }
                            self.print("ID\tSTATE\tBYTES_DONE\tBYTES_TOTAL\tNAME\tERROR");
                            for job in jobs {
                                self.print(format!(
                                    "{}\t{}\t{}\t{}\t{}\t{}",
                                    job.id,
                                    job.state,
                                    job.bytes_done,
                                    job.bytes_total,
                                    job.name,
                                    job.error.as_deref().unwrap_or("")
                                ));
                            }
                            complete = true;
                        }
                    }
                    Event::Thumb { post_id, path } => {
                        if matches!(command, CliCommand::Preview(id) if id == post_id) {
                            self.print(path.display().to_string());
                            complete = true;
                        }
                    }
                    Event::GroupRows { .. } | Event::PostsPage { .. } => unreachable!(),
                    Event::HeaderFetchFinished { group_id, error } => {
                        if matches!(&command, CliCommand::Sync { group_id: requested, .. } if *requested == group_id)
                        {
                            if let Some(error) = error {
                                return Err(self.scrub(&error));
                            }
                            complete = true;
                        }
                    }
                    Event::HeaderFetchCancelled { group_id } => {
                        if matches!(&command, CliCommand::Sync { group_id: requested, .. } if *requested == group_id)
                        {
                            let progress = header_progress.map_or_else(
                                String::new,
                                |(received, stored, _, _)| {
                                    format!("; {received} headers received, {stored} stored")
                                },
                            );
                            eprintln!("headers group {group_id}: cancelled{progress}");
                            return Ok(());
                        }
                    }
                    Event::HeaderProgress {
                        group_id,
                        received,
                        stored,
                        completed,
                        total,
                        first,
                        last,
                    } => {
                        if matches!(&command, CliCommand::Sync { group_id: requested, .. } if *requested == group_id)
                        {
                            header_progress = Some((received, stored, first, last));
                            eprintln!("headers group {group_id}: {completed}/{total} article slots scanned, {received} received, {stored} stored; articles {first}-{last}");
                        }
                    }
                    Event::MediaCached { .. }
                    | Event::GroupUnsubscribed { .. }
                    | Event::PostsDatePage { .. }
                    | Event::PostsDateFailed { .. }
                    | Event::MediaCacheFailed { .. }
                    | Event::PreviewMediaReady { .. }
                    | Event::PreviewMediaProgress { .. }
                    | Event::PreviewMediaFailed { .. }
                    | Event::MediaSaved { .. }
                    | Event::MediaSaveFailed { .. }
                    | Event::NetworkStats { .. }
                    | Event::NetworkTraffic { .. }
                    | Event::ConnectionStats { .. }
                    | Event::DownloadDeleted { .. }
                    | Event::DownloadDeleteFailed { .. }
                    | Event::ArticleContent { .. }
                    | Event::ArticleReadFailed { .. } => {}
                    Event::PostsFailed {
                        group_id,
                        search,
                        after,
                        error,
                    } => {
                        if after == last_post_key
                            && matches!(&command, CliCommand::Posts { group_id: requested, search: requested_search } if *requested == group_id && *requested_search == search)
                        {
                            return Err(self.scrub(&error));
                        }
                    }
                }
            }
            if complete {
                return Ok(());
            }
            if Instant::now() >= deadline {
                let progress = header_progress
                    .map(|(received, stored, first, last)| {
                        format!(
                            "; {received} headers received, {stored} stored from {first}-{last}"
                        )
                    })
                    .unwrap_or_default();
                return Err(format!(
                    "command timed out after {timeout} seconds{progress}"
                ));
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }
}
