//! Bulk queue construction uses one sequential header scan and a SQLite
//! temporary lookup, rather than retaining a segment vector for every post.
use super::*;
use crate::grouper::parse_subject;
use crate::osf::HeaderStore;

impl Catalog {
    #[allow(clippy::too_many_arguments)]
    pub fn queue_group_headers(
        &self,
        headers: &HeaderStore,
        server_id: i64,
        group_id: i64,
        group: &str,
        limit: Option<u64>,
        search: Option<&str>,
        downloads: &Path,
    ) -> Result<u64, Error> {
        if limit == Some(0) {
            return Ok(0);
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let subscription: Option<(i64, String)> = tx
            .query_row(
                "SELECT server_id, name FROM group_sub WHERE id=?1",
                [group_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if subscription
            .as_ref()
            .is_none_or(|(id, name)| *id != server_id || name != group)
        {
            return Err(Error::Header(
                "queue group does not match its server".into(),
            ));
        }
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS queue_selection (
                group_key TEXT PRIMARY KEY, job_id INTEGER NOT NULL, has_payload INTEGER NOT NULL
             ) WITHOUT ROWID; DELETE FROM queue_selection;",
        )?;
        let like = search.map(super::posts::like_contains);
        let mut after = (i64::MAX, i64::MAX);
        let mut remaining = limit.unwrap_or(u64::MAX);
        while remaining > 0 {
            // Bound Rust allocations even when the user queues the whole group.
            let rows = {
                let mut stmt = tx.prepare_cached(
                    "SELECT id, group_key, title, posted_at, poster,
                       EXISTS(SELECT 1 FROM file WHERE post_id=post.id AND kind!='par2') FROM post
                     WHERE group_id=?1 AND (posted_at, id) < (?2, ?3)
                       AND (?4 IS NULL OR title LIKE ?4 ESCAPE '\\')
                     ORDER BY posted_at DESC, id DESC LIMIT ?5",
                )?;
                let rows = stmt.query_map(
                    params![group_id, after.0, after.1, like, remaining.min(80)],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, bool>(5)?,
                        ))
                    },
                )?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            if rows.is_empty() {
                break;
            }
            remaining -= rows.len() as u64;
            for (id, group_key, title, posted_at, poster, has_payload) in rows {
                after = (posted_at, id);
                let clean: String = title
                    .chars()
                    .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ' '))
                    .collect();
                let clean = clean.trim();
                let name = if clean.is_empty() { "article" } else { clean };
                let dest = downloads.join(format!("{name}-{id}"));
                let dest = dest.display().to_string();
                tx.prepare_cached(
                    "INSERT INTO job (post_id, name, dest_dir, dest_path, state, bytes_done, bytes_total, error, password)
                     VALUES (?1, ?2, ?3, ?3, 'queued', 0, 0, NULL, NULL)",
                )?.execute(params![id, title, dest])?;
                let job_id = tx.last_insert_rowid();
                super::repair::save_context(
                    &tx,
                    job_id,
                    &JobRecoveryContext {
                        server_id: Some(server_id),
                        group_id: Some(group_id),
                        group: group.into(),
                        poster,
                        group_key: group_key.clone(),
                        posted_at: Some(posted_at),
                        set_id: None,
                        discovery_exhausted: false,
                        discovery_ranges: Vec::new(),
                    },
                )?;
                tx.prepare_cached("INSERT INTO queue_selection VALUES (?1, ?2, ?3)")?
                    .execute(params![group_key, job_id, has_payload])?;
            }
        }
        let selected: i64 =
            tx.query_row("SELECT COUNT(*) FROM queue_selection", [], |row| row.get(0))?;
        if selected == 0 {
            tx.commit()?;
            return Ok(0);
        }
        let mut failure = None;
        let bad = headers.for_each(server_id, group, |record| {
            if failure.is_some() { return; }
            let insert = (|| -> Result<(), Error> {
                let parsed = parse_subject(&record.subject, &record.from);
                let selected: Option<(i64,bool)> = tx.prepare_cached(
                    "SELECT job_id,has_payload FROM queue_selection WHERE group_key=?1",
                )?.query_row([&parsed.group_key], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
                let Some((job_id,has_payload)) = selected else { return Ok(()); };
                let filename = parsed.filename.unwrap_or(parsed.title);
                if !super::queue::valid_filename(&filename) {
                    return Err(Error::Header("download filename must be a single portable file name".into()));
                }
                if let Some(blocks) = recovery_volume_blocks(&filename).filter(|_| has_payload) {
                    super::repair::add_candidate(&tx,job_id,&NewSegment {
                        filename,part:i64::from(parsed.part.unwrap_or(1)),message_id:record.message_id.clone(),
                        begin:0,end:0,article:i64::try_from(record.article).ok(),
                    },Some(blocks),i64::from(parsed.total.unwrap_or(1)))?;
                    return Ok(());
                }
                tx.prepare_cached(
                    "INSERT INTO segment (job_id, filename, part, message_id, begin, end, article, state, server_id)
                     VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, 'pending', NULL)
                     ON CONFLICT(job_id, message_id) DO NOTHING",
                )?.execute(params![job_id, filename, parsed.part.unwrap_or(1), record.message_id,
                    i64::try_from(record.article).ok()])?;
                Ok(())
            })();
            if let Err(error) = insert { failure = Some(error); }
        })?;
        if let Some(error) = failure {
            return Err(error);
        }
        if !bad.is_empty() {
            return Err(Error::Header(
                "cannot queue a group with damaged header blocks".into(),
            ));
        }
        tx.execute(
            "DELETE FROM job WHERE id IN (SELECT job_id FROM queue_selection)
             AND NOT EXISTS (SELECT 1 FROM segment WHERE job_id=job.id)
             AND NOT EXISTS (SELECT 1 FROM recovery_candidate WHERE job_id=job.id)",
            [],
        )?;
        tx.execute(
            "UPDATE job SET dest_path=dest_dir || ?1 ||
                COALESCE((SELECT filename FROM segment WHERE job_id=job.id ORDER BY filename, part LIMIT 1),
                         (SELECT filename FROM recovery_candidate WHERE job_id=job.id ORDER BY filename, part LIMIT 1))
             WHERE id IN (SELECT job_id FROM queue_selection)",
            [std::path::MAIN_SEPARATOR.to_string()],
        )?;
        let queued: u64 = tx.query_row(
            "SELECT COUNT(*) FROM job WHERE id IN (SELECT job_id FROM queue_selection)",
            [],
            |row| row.get(0),
        )?;
        tx.execute("DELETE FROM queue_selection", [])?;
        tx.commit()?;
        Ok(queued)
    }
}
