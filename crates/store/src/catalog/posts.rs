use super::*;

impl Catalog {
    pub fn upsert_posts(&self, group_id: i64, posts: &[IncomingPost]) -> Result<(), Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for post in posts {
            let file_count = post.files.len() as i64;
            let total_bytes = post.files.iter().map(|file| file.size).sum::<i64>();
            let segment_count = post
                .files
                .iter()
                .map(|file| file.expected_segments)
                .sum::<i64>();
            let incomplete = post.files.iter().any(|file| file.expected_segments == 0);
            let has_image = post.files.iter().any(|file| file.kind == FileKind::Image);
            let has_par2 = post.files.iter().any(|file| file.kind == FileKind::Par2);
            let mut flags = 0i64;
            if incomplete {
                flags |= 1;
            }
            if has_image {
                flags |= 2;
            }
            if has_par2 {
                flags |= 4;
            }
            tx.prepare_cached(
                "INSERT INTO post (group_id, group_key, title, poster, posted_at, total_bytes, file_count, segment_count, flags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(group_id, group_key) DO UPDATE SET
                    title=excluded.title,
                    poster=excluded.poster,
                    posted_at=excluded.posted_at,
                    total_bytes=excluded.total_bytes,
                    file_count=excluded.file_count,
                    segment_count=excluded.segment_count,
                    flags=excluded.flags")?.execute(params![
                    group_id,
                    post.group_key,
                    post.title,
                    post.poster,
                    post.posted_at,
                    total_bytes,
                    file_count,
                    segment_count,
                    flags,
                ],
            )?;
            let post_id: i64 = tx
                .prepare_cached("SELECT id FROM post WHERE group_id=?1 AND group_key=?2")?
                .query_row(params![group_id, post.group_key], |row| row.get(0))?;
            if post.files.is_empty() {
                tx.execute("DELETE FROM file WHERE post_id=?1", [post_id])?;
            } else {
                // JSON gives one bound parameter regardless of file count;
                // variable-length SQL hit SQLite's parameter limit for very
                // large posts and could not reuse a prepared statement.
                let names: Vec<_> = post.files.iter().map(|file| file.name.as_str()).collect();
                let names =
                    serde_json::to_string(&names).map_err(|err| Error::Header(err.to_string()))?;
                tx.prepare_cached(
                    "DELETE FROM file WHERE post_id=?1 AND name NOT IN (SELECT value FROM json_each(?2))",
                )?.execute(params![post_id, names])?;
            }
            for file in &post.files {
                tx.prepare_cached(
                    "INSERT INTO file (post_id, name, size, expected_segments, kind)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(post_id, name) DO UPDATE SET
                        size=excluded.size,
                        expected_segments=excluded.expected_segments,
                        kind=excluded.kind",
                )?
                .execute(params![
                    post_id,
                    file.name,
                    file.size,
                    file.expected_segments,
                    file.kind.as_str(),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Adds files onto an existing article. Sizes accumulate and segment totals
    /// take the max, so a sync can flush a article in more than one batch.
    pub fn merge_posts(&self, group_id: i64, posts: &[IncomingPost]) -> Result<(), Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for post in posts {
            tx.prepare_cached(
                "INSERT INTO post (group_id, group_key, title, poster, posted_at, total_bytes, file_count, segment_count, flags)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 0, 0)
                 ON CONFLICT(group_id, group_key) DO UPDATE SET
                    title=excluded.title,
                    poster=excluded.poster,
                    posted_at=CASE WHEN excluded.posted_at > post.posted_at THEN excluded.posted_at ELSE post.posted_at END")?.execute(params![group_id, post.group_key, post.title, post.poster, post.posted_at],
            )?;
            let post_id: i64 = tx
                .prepare_cached("SELECT id FROM post WHERE group_id=?1 AND group_key=?2")?
                .query_row(params![group_id, post.group_key], |row| row.get(0))?;
            for file in &post.files {
                if let Some(alias) = super::file_alias::legacy_alias(post, &file.name) {
                    tx.prepare_cached(
                        "UPDATE file SET name=?1 WHERE post_id=?2 AND name=?3
                         AND NOT EXISTS (SELECT 1 FROM file WHERE post_id=?2 AND name=?1)",
                    )?
                    .execute(params![file.name, post_id, alias])?;
                }
                tx.prepare_cached(
                    "INSERT INTO file (post_id, name, size, expected_segments, kind)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(post_id, name) DO UPDATE SET
                        size = size + excluded.size,
                        expected_segments = MAX(expected_segments, excluded.expected_segments),
                        kind = excluded.kind",
                )?
                .execute(params![
                    post_id,
                    file.name,
                    file.size,
                    file.expected_segments,
                    file.kind.as_str(),
                ])?;
            }
            recompute_post(&tx, post_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Prepare absolute snapshots for an idempotent OSF/catalog journal replay.
    pub(crate) fn merged_post_snapshots(
        &self,
        group_id: i64,
        posts: &[IncomingPost],
    ) -> Result<Vec<IncomingPost>, Error> {
        let conn = self.lock();
        let mut snapshots = Vec::with_capacity(posts.len());
        for post in posts {
            let existing: Option<(i64, i64)> = conn
                .prepare_cached(
                    "SELECT id, posted_at FROM post WHERE group_id=?1 AND group_key=?2",
                )?
                .query_row(params![group_id, post.group_key], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .optional()?;
            let mut snapshot = post.clone();
            let mut files = std::collections::BTreeMap::new();
            if let Some((post_id, posted_at)) = existing {
                snapshot.posted_at = snapshot.posted_at.max(posted_at);
                let mut stmt = conn.prepare_cached(
                    "SELECT name, size, expected_segments, kind FROM file WHERE post_id=?1",
                )?;
                let rows = stmt.query_map([post_id], |row| {
                    Ok(IncomingFile {
                        name: row.get(0)?,
                        size: row.get(1)?,
                        expected_segments: row.get(2)?,
                        kind: FileKind::parse(&row.get::<_, String>(3)?),
                    })
                })?;
                for file in rows {
                    let file = file?;
                    files.insert(file.name.clone(), file);
                }
            }
            for incoming in &post.files {
                if !files.contains_key(&incoming.name) {
                    if let Some(alias) = super::file_alias::legacy_alias(post, &incoming.name) {
                        if let Some(mut existing) = files.remove(&alias) {
                            existing.name = incoming.name.clone();
                            files.insert(incoming.name.clone(), existing);
                        }
                    }
                }
                let file = files.entry(incoming.name.clone()).or_insert(IncomingFile {
                    name: incoming.name.clone(),
                    size: 0,
                    expected_segments: 0,
                    kind: incoming.kind,
                });
                file.size = file
                    .size
                    .checked_add(incoming.size)
                    .ok_or_else(|| Error::Header("post file size overflow".into()))?;
                file.expected_segments = file.expected_segments.max(incoming.expected_segments);
                file.kind = incoming.kind;
            }
            snapshot.files = files.into_values().collect();
            snapshots.push(snapshot);
        }
        Ok(snapshots)
    }

    pub fn page_posts(
        &self,
        group_id: i64,
        after: Option<(i64, i64)>,
        limit: u32,
        search: Option<&str>,
    ) -> Result<Vec<PostRow>, Error> {
        self.page_posts_date_range(group_id, after, limit, search, None, None)
    }

    /// Date bounds are inclusive UTC timestamps; filtering precedes keyset pagination.
    pub fn page_posts_date_range(
        &self,
        group_id: i64,
        after: Option<(i64, i64)>,
        limit: u32,
        search: Option<&str>,
        from: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<PostRow>, Error> {
        let conn = self.lock();
        let like = search.map(like_contains);
        let (after_posted, after_id) = after.unwrap_or((i64::MAX, i64::MAX));
        let mut stmt = conn.prepare_cached(
            "SELECT id, group_id, group_key, title, poster, posted_at, total_bytes, file_count, segment_count, flags,
               EXISTS(SELECT 1 FROM file WHERE file.post_id=post.id AND
                   (file.kind='video' OR (file.kind='data' AND (
                     lower(file.name) GLOB '*.mp4' OR lower(file.name) GLOB '*.mkv' OR
                     lower(file.name) GLOB '*.webm' OR lower(file.name) GLOB '*.mov' OR
                     lower(file.name) GLOB '*.avi' OR lower(file.name) GLOB '*.m4v' OR
                     lower(file.name) GLOB '*.wmv' OR lower(file.name) GLOB '*.mpg' OR
                     lower(file.name) GLOB '*.mpeg' OR lower(file.name) GLOB '*.3gp' OR
                     lower(file.name) GLOB '*.m2ts' OR lower(file.name) GLOB '*.mts'))))
             FROM post
             WHERE group_id=?1
               AND (posted_at, id) < (?2, ?3)
               AND (?4 IS NULL OR title LIKE ?4 ESCAPE '\\')
               AND (?6 IS NULL OR posted_at >= ?6)
               AND (?7 IS NULL OR posted_at <= ?7)
               AND ((?6 IS NULL AND ?7 IS NULL) OR posted_at > 0)
             ORDER BY posted_at DESC, id DESC
             LIMIT ?5",
        )?;
        let rows = stmt.query_map(
            params![group_id, after_posted, after_id, like, limit, from, until],
            post_from_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    pub fn post_by_id(&self, id: i64) -> Result<Option<PostRow>, Error> {
        self.lock().query_row(
            "SELECT id, group_id, group_key, title, poster, posted_at, total_bytes, file_count, segment_count, flags,
               EXISTS(SELECT 1 FROM file WHERE file.post_id=post.id AND
                   (file.kind='video' OR (file.kind='data' AND (
                     lower(file.name) GLOB '*.mp4' OR lower(file.name) GLOB '*.mkv' OR
                     lower(file.name) GLOB '*.webm' OR lower(file.name) GLOB '*.mov' OR
                     lower(file.name) GLOB '*.avi' OR lower(file.name) GLOB '*.m4v' OR
                     lower(file.name) GLOB '*.wmv' OR lower(file.name) GLOB '*.mpg' OR
                     lower(file.name) GLOB '*.mpeg' OR lower(file.name) GLOB '*.3gp' OR
                     lower(file.name) GLOB '*.m2ts' OR lower(file.name) GLOB '*.mts')))) FROM post WHERE id=?1",
            [id], post_from_row,
        ).optional().map_err(Error::from)
    }

    pub fn files_for_post(&self, post_id: i64) -> Result<Vec<FileRow>, Error> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, post_id, name, size, expected_segments, kind FROM file WHERE post_id=?1 ORDER BY name",
        )?;
        let rows = stmt.query_map([post_id], |row| {
            let name: String = row.get(2)?;
            let stored_kind = FileKind::parse(&row.get::<_, String>(5)?);
            let kind = if stored_kind == FileKind::Data
                && crate::grouper::classify(&name) == FileKind::Video
            {
                FileKind::Video
            } else {
                stored_kind
            };
            Ok(FileRow {
                id: row.get(0)?,
                post_id: row.get(1)?,
                name,
                size: row.get(3)?,
                expected_segments: row.get(4)?,
                kind,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    pub fn post_count(&self, group_id: i64) -> Result<i64, Error> {
        let n = self.lock().query_row(
            "SELECT COUNT(*) FROM post WHERE group_id=?1",
            [group_id],
            |row| row.get(0),
        )?;
        Ok(n)
    }
}

fn post_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PostRow> {
    let flags: i64 = row.get(9)?;
    Ok(PostRow {
        id: row.get(0)?,
        group_id: row.get(1)?,
        group_key: row.get(2)?,
        title: row.get(3)?,
        poster: row.get(4)?,
        posted_at: row.get(5)?,
        total_bytes: row.get(6)?,
        file_count: row.get(7)?,
        segment_count: row.get(8)?,
        incomplete: flags & 1 != 0,
        has_image: flags & 2 != 0,
        has_video: row.get(10)?,
        has_par2: flags & 4 != 0,
    })
}

fn recompute_post(tx: &rusqlite::Transaction<'_>, post_id: i64) -> Result<(), Error> {
    let (total, count, segments, incomplete, has_image, has_par2): (
        i64,
        i64,
        i64,
        bool,
        bool,
        bool,
    ) = tx
        .prepare_cached(
            "SELECT COALESCE(SUM(size), 0), COUNT(*), COALESCE(SUM(expected_segments), 0),
                    COALESCE(MAX(expected_segments = 0), 0),
                    COALESCE(MAX(kind = 'image'), 0), COALESCE(MAX(kind = 'par2'), 0)
             FROM file WHERE post_id=?1",
        )?
        .query_row([post_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?;
    let mut flags = 0i64;
    if incomplete {
        flags |= 1;
    }
    if has_image {
        flags |= 2;
    }
    if has_par2 {
        flags |= 4;
    }
    tx.prepare_cached(
        "UPDATE post SET total_bytes=?1, file_count=?2, segment_count=?3, flags=?4 WHERE id=?5",
    )?
    .execute(params![total, count, segments, flags, post_id])?;
    Ok(())
}

pub(super) fn like_contains(search: &str) -> String {
    let mut escaped = String::new();
    for ch in search.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    format!("%{escaped}%")
}
