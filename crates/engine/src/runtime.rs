//! Tokio worker. The UI sends [`Command`]s and polls [`Event`]s. The egui
//! mutex is never held across an await because this task does not touch it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use nntp::Pool;
use store::{Catalog, GroupRow, HeaderStore, ServerRow};
use tokio::sync::mpsc::UnboundedSender;

pub use api::{Command, ConnectionStats, Event, Handle, ListedGroup};

mod api;
mod article;
mod completeness;
mod downloads;
mod ensure_headers;
mod finalize;
mod header_wait;
mod manage_downloads;
mod media;
mod previews;
mod queue;
mod ram;
mod recovery;
mod recovery_discovery;
mod scheduler;
mod servers;
mod startup;
mod sync;
mod telemetry;
mod unsubscribe;
mod uu_downloads;

struct Worker {
    catalog: Catalog,
    headers: HeaderStore,
    data_dir: PathBuf,
    pools: HashMap<i64, Arc<Pool>>,
    network_timeout_secs: u64,
    network_timeout_dirty: bool,
    active_header_fetch: Option<i64>,
    header_fetch_cancelled: bool,
    transfers: tokio::task::JoinSet<scheduler::WindowResult>,
    in_flight: HashMap<i64, (i64, u64)>,
    transfer_generation: HashMap<i64, u64>,
    preview_context: u64,
    preview_tasks: tokio::task::JoinSet<ram::PreviewResult>,
    preview_active: HashMap<tokio::task::Id, ram::ActivePreview>,
    preview_queue: VecDeque<ram::PreviewIntent>,
    preview_wanted: Option<HashSet<i64>>,
    preview_pending: HashSet<(i64, u64)>,
    preview_cache: VecDeque<ram::CachedMedia>,
    paused: HashSet<i64>,
    cancelled: HashSet<i64>,
    cmd_rx: Receiver<Command>,
    deferred: VecDeque<Command>,
    evt: UnboundedSender<Event>,
    downloads: PathBuf,
}

async fn run_worker(data_dir: PathBuf, cmd_rx: Receiver<Command>, evt: UnboundedSender<Event>) {
    // Keep the file open for this worker's entire lifetime. Removing the lock
    // file would allow another process to lock a different inode.
    let _data_lock = match startup::lock_data_dir(&data_dir) {
        Ok(lock) => lock,
        Err(err) => {
            let _ = evt.send(Event::Notice(format!("startup: {err}")));
            return;
        }
    };
    let catalog = match Catalog::open(data_dir.join("catalog.sqlite")) {
        Ok(catalog) => catalog,
        Err(err) => {
            let _ = evt.send(Event::Notice(format!("catalog: {err}")));
            return;
        }
    };
    let headers = match HeaderStore::open(data_dir.join("headers")) {
        Ok(headers) => headers,
        Err(err) => {
            let _ = evt.send(Event::Notice(format!("headers: {err}")));
            return;
        }
    };
    if let Err(err) = headers.recover_catalog(&catalog) {
        let _ = evt.send(Event::Notice(format!("recover headers: {err}")));
        return;
    }
    let downloads = data_dir.join("downloads");
    let _ = std::fs::create_dir_all(&downloads);
    set_dir_mode(&downloads);
    let mut worker = Worker {
        catalog,
        headers,
        data_dir,
        pools: HashMap::new(),
        network_timeout_secs: 8,
        network_timeout_dirty: false,
        active_header_fetch: None,
        header_fetch_cancelled: false,
        transfers: tokio::task::JoinSet::new(),
        in_flight: HashMap::new(),
        transfer_generation: HashMap::new(),
        preview_context: 0,
        preview_tasks: tokio::task::JoinSet::new(),
        preview_active: HashMap::new(),
        preview_queue: VecDeque::new(),
        preview_wanted: None,
        preview_pending: HashSet::new(),
        preview_cache: VecDeque::new(),
        paused: HashSet::new(),
        cancelled: HashSet::new(),
        cmd_rx,
        deferred: VecDeque::new(),
        evt,
        downloads,
    };
    worker.stop_legacy_media_jobs();
    worker.cleanup_completed_uu_stages();
    worker.emit_servers();
    worker.emit_jobs();
    loop {
        while let Some(command) = worker.deferred.pop_front() {
            worker.on_command(command).await;
        }
        loop {
            match worker.cmd_rx.try_recv() {
                Ok(command) => worker.on_command(command).await,
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return,
            }
        }
        let worked = worker.drive_one().await;
        if !worked {
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
}

impl Worker {
    async fn on_command(&mut self, command: Command) {
        match command {
            Command::ConfigureStorage { downloads_dir } => {
                if !downloads_dir.is_absolute() {
                    self.notice("storage failed: downloads directory must be absolute".into());
                } else if let Err(error) = std::fs::create_dir_all(&downloads_dir) {
                    self.notice(format!("storage failed: {error}"));
                } else {
                    self.downloads = downloads_dir;
                }
            }
            Command::ConfigureNetwork { timeout_secs } => {
                let timeout_secs = timeout_secs.clamp(2, 120);
                if self.network_timeout_secs != timeout_secs {
                    self.network_timeout_secs = timeout_secs;
                    self.network_timeout_dirty = true;
                }
            }
            Command::Refresh => {
                self.emit_servers();
                self.emit_jobs();
            }
            Command::SaveServer(server) => match self.catalog.insert_server(&server) {
                Ok(_) => self.emit_servers(),
                Err(err) => self.notice(format!("save server: {err}")),
            },
            Command::SaveServerEnabled { server, enabled } => {
                match self.catalog.insert_server_enabled(&server, enabled) {
                    Ok(_) => self.emit_servers(),
                    Err(err) => self.notice(format!("save server: {err}")),
                }
            }
            Command::UpdateServer {
                id,
                server,
                enabled,
            } => {
                self.pools.remove(&id);
                if let Err(err) = self.catalog.update_server(id, &server, enabled) {
                    self.notice(format!("update server: {err}"));
                }
                self.emit_servers();
            }
            Command::DeleteServer(id) => {
                self.delete_server(id);
                self.emit_servers();
            }
            Command::TestServer(id) => self.test_server(id).await,
            Command::Subscribe { server_id, name } => {
                if let Err(err) = self.catalog.subscribe(server_id, &name) {
                    self.notice(format!("subscribe: {err}"));
                }
                self.emit_groups(server_id);
            }
            Command::Unsubscribe { server_id, name } => {
                self.unsubscribe_group(server_id, &name);
            }
            Command::ListActive { server_id, wildmat } => {
                self.list_active(server_id, &wildmat).await
            }
            Command::SelectGroup(id) => self.load_posts(id, None, ""),
            Command::ShowGroups(id) => self.emit_groups(id),
            Command::LoadPosts {
                group_id,
                after,
                search,
            } => self.load_posts(group_id, after, &search),
            Command::LoadPostsInDateRange {
                group_id,
                after,
                search,
                from,
                until,
            } => self.load_posts_in_date_range(group_id, after, &search, from, until),
            Command::SyncGroup(id) => self.sync_group(id).await,
            Command::CancelHeaderFetch(id) => self.cancel_header_fetch(id),
            Command::EnsureGroupHeaders(id) => self.ensure_group_headers(id).await,
            Command::FetchHeaders { group_id, limit } => self.fetch_headers(group_id, limit).await,
            Command::OpenPost(id) => self.open_post(id),
            Command::QueuePost { post_id } => self.queue_post(post_id),
            Command::QueueGroup {
                group_id,
                limit,
                search,
            } => self.queue_group(group_id, limit, &search).await,
            Command::ImportNzb { xml, dest } => self.import_nzb(&xml, &dest),
            Command::Pause(id) => self.pause(id),
            Command::Resume(id) => self.resume(id),
            Command::Cancel(id) => self.cancel(id),
            Command::DeleteDownload {
                job_id,
                remove_files,
            } => self.delete_download(job_id, remove_files),
            Command::Preview(id) => self.preview(id).await,
            Command::CacheMedia { post_id, max_bytes } => self.cache_media(post_id, max_bytes),
            Command::ReadArticle(id) => self.read_article(id).await,
            Command::PreviewMedia {
                post_id,
                max_bytes,
                generation,
            } => self.preview_media(post_id, max_bytes, generation),
            Command::PreviewContext(generation) => self.set_preview_context(generation),
            Command::RetainPreviewMedia {
                post_ids,
                generation,
            } => self.retain_preview_media(&post_ids, generation),
            Command::SavePreviewMedia {
                post_id,
                generation,
            } => self.save_preview_media(post_id, generation),
        }
    }

    fn open_post(&self, post_id: i64) {
        if self.catalog.post_by_id(post_id).ok().flatten().is_none() {
            self.notice("files: article was not found".into());
            return;
        }
        self.emit_cached_media(post_id);
        match self.catalog.files_for_post(post_id) {
            Ok(files) => {
                let _ = self.evt.send(Event::Files { post_id, files });
            }
            Err(err) => self.notice(format!("files: {err}")),
        }
    }

    fn emit_servers(&self) {
        if let Ok(servers) = self.catalog.list_servers() {
            let _ = self.evt.send(Event::Servers(servers));
        }
    }

    fn emit_groups(&self, server_id: i64) {
        match self.catalog.list_groups(server_id) {
            Ok(groups) => {
                let _ = self.evt.send(Event::GroupRows { server_id, groups });
            }
            Err(err) => self.notice(format!("groups: {err}")),
        }
    }

    fn emit_jobs(&self) {
        if let Ok(jobs) = self.catalog.list_jobs() {
            let _ = self.evt.send(Event::Jobs(jobs));
        }
    }

    fn notice(&self, text: String) {
        let _ = self.evt.send(Event::Notice(text));
    }

    fn server(&self, id: i64) -> Option<ServerRow> {
        self.catalog.server_by_id(id).ok().flatten()
    }

    fn find_group(&self, group_id: i64) -> Option<(ServerRow, GroupRow)> {
        let group = self.catalog.group_by_id(group_id).ok().flatten()?;
        if !group.subscribed {
            return None;
        }
        Some((self.server(group.server_id)?, group))
    }

    fn find_post(&self, post_id: i64) -> Option<(ServerRow, GroupRow, store::PostRow)> {
        let post = self.catalog.post_by_id(post_id).ok().flatten()?;
        // Unsubscribe deletes article metadata; existing downloads use their
        // independent job/segment rows instead of this lookup.
        let group = self.catalog.group_by_id(post.group_id).ok().flatten()?;
        Some((self.server(group.server_id)?, group, post))
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn set_dir_mode(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    let _ = path;
}

#[cfg(test)]
mod tests;
