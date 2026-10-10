use super::*;

impl Catalog {
    pub fn create_job(
        &self,
        post_id: Option<i64>,
        name: &str,
        dest_dir: &Path,
        password: Option<&str>,
        segments: &[NewSegment],
    ) -> Result<i64, Error> {
        self.create_job_with_recovery(post_id, name, dest_dir, password, segments, None, &[])
    }

    /// Queue active segments and on-demand recovery metadata atomically.
    #[allow(clippy::too_many_arguments)]
    pub fn create_job_with_recovery(
        &self,
        post_id: Option<i64>,
        name: &str,
        dest_dir: &Path,
        password: Option<&str>,
        segments: &[NewSegment],
        context: Option<&JobRecoveryContext>,
        candidates: &[(NewSegment, Option<i64>, i64)],
    ) -> Result<i64, Error> {
        for segment in segments {
            super::repair::validate_segment(segment)?;
        }
        let dest_dir_text = dest_dir.display().to_string();
        let filename = segments
            .first()
            .map(|segment| segment.filename.as_str())
            .unwrap_or(name);
        let dest_path = dest_dir.join(filename).display().to_string();
        let bytes_total = segments
            .iter()
            .try_fold(0i64, |total, segment| {
                total.checked_add(segment.end - segment.begin)
            })
            .ok_or_else(|| Error::Header("download size exceeds catalog range".into()))?;
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO job (post_id, name, dest_dir, dest_path, state, bytes_done, bytes_total, error, password)
             VALUES (?1, ?2, ?3, ?4, 'queued', 0, ?5, NULL, ?6)",
            params![post_id, name, dest_dir_text, dest_path, bytes_total, password],
        )?;
        let job_id = tx.last_insert_rowid();
        for segment in segments {
            tx.prepare_cached(
                "INSERT INTO segment (job_id, filename, part, message_id, begin, end, article, state, server_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL)")?.execute(params![
                    job_id,
                    segment.filename,
                    segment.part,
                    segment.message_id,
                    segment.begin,
                    segment.end,
                    segment.article,
                ],
            )?;
        }
        if let Some(context) = context {
            super::repair::save_context(&tx, job_id, context)?;
        }
        for (segment, blocks, parts) in candidates {
            super::repair::add_candidate(&tx, job_id, segment, *blocks, *parts)?;
        }
        tx.commit()?;
        Ok(job_id)
    }

    pub fn list_jobs(&self) -> Result<Vec<JobRow>, Error> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, post_id, name, dest_dir, dest_path, state, bytes_done, bytes_total, error, password
             FROM job ORDER BY id",
        )?;
        let rows = stmt.query_map([], job_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    /// Read the current transfer counters/state without materializing the queue.
    pub fn job_by_id(&self, id: i64) -> Result<Option<JobRow>, Error> {
        self.lock()
            .prepare_cached(
                "SELECT id, post_id, name, dest_dir, dest_path, state, bytes_done, bytes_total, error, password
                 FROM job WHERE id=?1",
            )?
            .query_row([id], job_from_row)
            .optional()
            .map_err(Error::from)
    }

    /// Remove a queue/history entry and its segments atomically. Article
    /// metadata and saved files are retained; filesystem removal is explicit.
    /// Returns false if the job was already absent.
    pub fn delete_job(&self, id: i64) -> Result<bool, Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let removed = tx.execute("DELETE FROM job WHERE id=?1", [id])? > 0;
        tx.commit()?;
        Ok(removed)
    }

    /// Record an explicitly saved preview as completed download history. The
    /// caller writes the file first; preview browsing never calls this method.
    pub fn record_saved_file(
        &self,
        post_id: i64,
        name: &str,
        path: &Path,
        bytes: u64,
    ) -> Result<i64, Error> {
        let filename = path.file_name().and_then(|part| part.to_str());
        let parent = path.parent().filter(|parent| parent.is_absolute());
        if !path.is_absolute()
            || filename.is_none_or(|filename| !valid_filename(filename))
            || parent.is_none()
            || name.trim().is_empty()
            || name.chars().any(char::is_control)
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(Error::Header(
                "saved file must have an absolute portable path and a valid display name".into(),
            ));
        }
        let dest_path = path
            .to_str()
            .ok_or_else(|| Error::Header("saved file path is not UTF-8".into()))?;
        let dest_dir = parent
            .and_then(Path::to_str)
            .ok_or_else(|| Error::Header("saved file directory is not UTF-8".into()))?;
        let size = i64::try_from(bytes)
            .map_err(|_| Error::Header("saved file size exceeds catalog range".into()))?;
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO job (post_id,name,dest_dir,dest_path,state,bytes_done,bytes_total,error,password)
             VALUES (?1,?2,?3,?4,'done',?5,?5,NULL,NULL)",
            params![post_id, name, dest_dir, dest_path, size],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    pub fn set_job_state(&self, id: i64, state: &str, error: Option<&str>) -> Result<(), Error> {
        self.lock().execute(
            "UPDATE job SET state=?1, error=?2 WHERE id=?3",
            params![state, error, id],
        )?;
        Ok(())
    }

    pub fn set_job_progress(
        &self,
        id: i64,
        bytes_done: i64,
        bytes_total: i64,
    ) -> Result<(), Error> {
        self.lock().execute(
            "UPDATE job SET bytes_done=?1, bytes_total=?2 WHERE id=?3",
            params![bytes_done, bytes_total, id],
        )?;
        Ok(())
    }

    pub fn segments_for_job(&self, job_id: i64) -> Result<Vec<SegmentRow>, Error> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, job_id, filename, part, message_id, begin, end, article, state, server_id
             FROM segment WHERE job_id=?1 ORDER BY filename, part",
        )?;
        let rows = stmt.query_map([job_id], segment_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    /// Persist actual decoded offsets before a segment is marked written.
    pub fn set_segment_bounds(&self, id: i64, begin: i64, end: i64) -> Result<(), Error> {
        if begin < 0 || end < begin {
            return Err(Error::Header("invalid decoded segment bounds".into()));
        }
        self.lock().execute(
            "UPDATE segment SET begin=?1, end=?2 WHERE id=?3",
            params![begin, end, id],
        )?;
        Ok(())
    }

    pub fn set_segment_state(
        &self,
        id: i64,
        state: &str,
        server_id: Option<i64>,
    ) -> Result<(), Error> {
        self.lock().execute(
            "UPDATE segment SET state=?1, server_id=COALESCE(?2, server_id) WHERE id=?3",
            params![state, server_id, id],
        )?;
        Ok(())
    }
}

fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        id: row.get(0)?,
        post_id: row.get(1)?,
        name: row.get(2)?,
        dest_dir: row.get(3)?,
        dest_path: row.get(4)?,
        state: row.get(5)?,
        bytes_done: row.get(6)?,
        bytes_total: row.get(7)?,
        error: row.get(8)?,
        password: row.get(9)?,
    })
}

fn segment_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SegmentRow> {
    Ok(SegmentRow {
        id: row.get(0)?,
        job_id: row.get(1)?,
        filename: row.get(2)?,
        part: row.get(3)?,
        message_id: row.get(4)?,
        begin: row.get(5)?,
        end: row.get(6)?,
        article: row.get(7)?,
        state: row.get(8)?,
        server_id: row.get(9)?,
    })
}

pub(super) fn valid_filename(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '\0', '*', '?', '"', '<', '>', '|'])
        && !name.ends_with(['.', ' '])
        && !name.chars().any(char::is_control)
}
