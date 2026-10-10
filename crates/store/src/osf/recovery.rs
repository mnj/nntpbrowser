//! Recoverable publication across immutable header files and SQLite.
use super::*;
use crate::catalog::{Catalog, IncomingPost};

const PENDING: &str = "pending_catalog.json";

#[derive(Serialize, Deserialize)]
struct PendingCatalog {
    server_id: i64,
    group_id: i64,
    group: String,
    low: u64,
    high: u64,
    posts: Vec<IncomingPost>,
}

impl HeaderStore {
    /// Commit a header batch and its grouped posts. A small durable journal
    /// makes replay idempotent if the process stops between the two stores.
    pub fn append_catalog_batch(
        &self,
        catalog: &Catalog,
        server_id: i64,
        group_id: i64,
        group: &str,
        records: &[Record],
        posts: &[IncomingPost],
    ) -> Result<(), Error> {
        if records.is_empty() {
            return Ok(());
        }
        let dir = self.group_dir(server_id, group)?;
        recover_pending(catalog, &dir)?;
        // Validate before journaling: a rejected overlapping retry must never
        // be mistaken for publication of this new batch during recovery.
        if records
            .windows(2)
            .any(|pair| pair[0].article >= pair[1].article)
        {
            return Err(Error::Order);
        }
        let low = records[0].article;
        let high = records.last().unwrap().article;
        if read_manifest(&dir)?
            .files
            .iter()
            .any(|file| low <= file.high && file.low <= high)
        {
            return Err(Error::Header("header batch overlaps a stored span".into()));
        }
        let Some(subscription) = catalog.group_by_id(group_id)? else {
            return Err(Error::Header("header batch group does not exist".into()));
        };
        if subscription.server_id != server_id || subscription.name != group {
            return Err(Error::Header(
                "header batch belongs to another group".into(),
            ));
        }
        let pending = PendingCatalog {
            server_id,
            group_id,
            group: group.into(),
            low: records[0].article,
            high: records.last().unwrap().article,
            posts: catalog.merged_post_snapshots(group_id, posts)?,
        };
        write_pending(&dir, &pending)?;
        self.append_range(server_id, group, records)?;
        catalog.upsert_posts(group_id, &pending.posts)?;
        fs::remove_file(dir.join(PENDING))?;
        Ok(())
    }

    /// Recover published header batches before exposing catalog snapshots.
    pub fn recover_catalog(&self, catalog: &Catalog) -> Result<usize, Error> {
        let mut recovered = 0;
        for server in fs::read_dir(&self.root)? {
            let server = server?;
            if !server.file_type()?.is_dir() {
                continue;
            }
            for group in fs::read_dir(server.path())? {
                let group = group?;
                if group.file_type()?.is_dir() && recover_pending(catalog, &group.path())? {
                    recovered += 1;
                }
            }
        }
        Ok(recovered)
    }
}

fn write_pending(dir: &Path, pending: &PendingCatalog) -> Result<(), Error> {
    let tmp = dir.join("pending_catalog.json.tmp");
    let bytes = serde_json::to_vec(pending).map_err(|err| Error::Header(err.to_string()))?;
    let mut file = create_file(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, dir.join(PENDING))?;
    Ok(())
}

fn recover_pending(catalog: &Catalog, dir: &Path) -> Result<bool, Error> {
    let path = dir.join(PENDING);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    let pending: PendingCatalog = serde_json::from_slice(&bytes)
        .map_err(|err| Error::Header(format!("catalog recovery journal: {err}")))?;
    let Some(group) = catalog.group_by_id(pending.group_id)? else {
        // The subscription/server may have been deleted since the failed batch.
        fs::remove_file(path)?;
        return Ok(false);
    };
    if group.server_id != pending.server_id
        || group.name != pending.group
        || dir.file_name().and_then(|name| name.to_str())
            != Some(group_slug(&pending.group).as_str())
        || dir
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some(pending.server_id.to_string().as_str())
    {
        return Err(Error::Header(
            "catalog journal belongs to another group".into(),
        ));
    }
    let published = read_manifest(dir)?
        .files
        .iter()
        .any(|file| file.low <= pending.low && file.high >= pending.high);
    if published {
        // Absolute sizes/files make repeated replay safe after a SQLite commit
        // followed by a crash before journal removal.
        catalog.upsert_posts(pending.group_id, &pending.posts)?;
    }
    fs::remove_file(path)?;
    Ok(published)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{IncomingFile, NewServer};
    use crate::grouper::FileKind;

    #[test]
    fn journal_handles_all_commit_boundaries_without_missing_or_duplicate_sizes() {
        let root = std::env::temp_dir().join(format!(
            "nntpbrowser-recovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let catalog = Catalog::open(root.join("catalog.sqlite")).unwrap();
        let store = HeaderStore::open(root.join("headers")).unwrap();
        let sid = catalog
            .insert_server(&NewServer {
                name: "test".into(),
                host: "localhost".into(),
                port: 119,
                tls: false,
                starttls: false,
                username: None,
                password: None,
                priority: 1,
                max_connections: 1,
                pipeline_depth: 1,
                fill: false,
            })
            .unwrap();
        let gid = catalog.subscribe(sid, "alt.test").unwrap();
        let dir = store.group_dir(sid, "alt.test").unwrap();
        let post = IncomingPost {
            group_key: "article".into(),
            title: "Article".into(),
            poster: "Poster".into(),
            posted_at: 1,
            files: vec![IncomingFile {
                name: "file.bin".into(),
                size: 10,
                expected_segments: 2,
                kind: FileKind::Data,
            }],
        };
        let record = |article| Record {
            article,
            date_unix: 1,
            date_raw: None,
            bytes: 10,
            lines: 1,
            message_id: format!("<{article}@test>"),
            subject: "Article".into(),
            from: "Poster".into(),
        };
        let stage = |article| PendingCatalog {
            server_id: sid,
            group_id: gid,
            group: "alt.test".into(),
            low: article,
            high: article,
            posts: catalog
                .merged_post_snapshots(gid, std::slice::from_ref(&post))
                .unwrap(),
        };
        // Crash before the header manifest is published: no catalog update.
        write_pending(&dir, &stage(1)).unwrap();
        assert_eq!(store.recover_catalog(&catalog).unwrap(), 0);
        assert_eq!(catalog.post_count(gid).unwrap(), 0);
        // Crash after OSF publication, before SQLite commit: restore catalog.
        write_pending(&dir, &stage(1)).unwrap();
        store.append_range(sid, "alt.test", &[record(1)]).unwrap();
        assert_eq!(store.recover_catalog(&catalog).unwrap(), 1);
        assert_eq!(
            catalog.page_posts(gid, None, 10, None).unwrap()[0].total_bytes,
            10
        );
        // Crash after SQLite commit, before removal: replay doesn't double count.
        let pending = stage(2);
        write_pending(&dir, &pending).unwrap();
        store.append_range(sid, "alt.test", &[record(2)]).unwrap();
        catalog.upsert_posts(gid, &pending.posts).unwrap();
        assert_eq!(store.recover_catalog(&catalog).unwrap(), 1);
        assert_eq!(
            catalog.page_posts(gid, None, 10, None).unwrap()[0].total_bytes,
            20
        );
        // Normal batch publication still accumulates correctly.
        store
            .append_catalog_batch(&catalog, sid, gid, "alt.test", &[record(3)], &[post])
            .unwrap();
        assert_eq!(
            catalog.page_posts(gid, None, 10, None).unwrap()[0].total_bytes,
            30
        );
        assert_eq!(store.recover_catalog(&catalog).unwrap(), 0);
        // A rejected duplicate append cannot leave a replayable double count.
        assert!(store
            .append_catalog_batch(
                &catalog,
                sid,
                gid,
                "alt.test",
                &[record(3)],
                &[IncomingPost {
                    group_key: "article".into(),
                    title: "Article".into(),
                    poster: "Poster".into(),
                    posted_at: 1,
                    files: vec![IncomingFile {
                        name: "file.bin".into(),
                        size: 10,
                        expected_segments: 2,
                        kind: FileKind::Data
                    }]
                }]
            )
            .is_err());
        assert_eq!(store.recover_catalog(&catalog).unwrap(), 0);
        assert_eq!(
            catalog.page_posts(gid, None, 10, None).unwrap()[0].total_bytes,
            30
        );

        drop(catalog);
        let _ = fs::remove_dir_all(root);
    }
}
