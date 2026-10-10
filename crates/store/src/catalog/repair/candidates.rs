use super::*;

impl Catalog {
    pub fn add_recovery_candidates(
        &self,
        job_id: i64,
        candidates: &[(NewSegment, Option<i64>, i64)],
    ) -> Result<usize, Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let mut added = 0;
        for (segment, blocks, parts) in candidates {
            added += add_candidate(&tx, job_id, segment, *blocks, *parts)?;
        }
        tx.commit()?;
        Ok(added)
    }

    pub fn recovery_candidates(&self, job_id: i64) -> Result<Vec<RecoveryCandidate>, Error> {
        let conn = self.lock();
        let mut statement = conn.prepare_cached(
            "SELECT id,job_id,filename,part,message_id,begin,end,article,recovery_blocks,expected_parts,state
             FROM recovery_candidate WHERE job_id=?1 ORDER BY filename,part,id")?;
        let rows = statement.query_map([job_id], candidate_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    /// Move pending auxiliary volumes out of the normal transfer queue. A
    /// single transaction copies their metadata then removes those rows;
    /// written/fetching/failed rows are never reset or deleted here.
    pub fn defer_job_segments(
        &self,
        job_id: i64,
        segments: &[(i64, Option<i64>, i64)],
    ) -> Result<usize, Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let mut deferred = 0;
        for (id, blocks, parts) in segments {
            let segment = tx
                .query_row(
                    "SELECT filename,part,message_id,begin,end,article FROM segment
                 WHERE id=?1 AND job_id=?2 AND state='pending'",
                    params![id, job_id],
                    new_segment_from_row,
                )
                .optional()?;
            if let Some(segment) = segment {
                add_candidate(&tx, job_id, &segment, *blocks, *parts)?;
                // Re-running legacy initialization must not defer a volume
                // already activated for recovery.
                let state: String = tx.query_row(
                    "SELECT state FROM recovery_candidate WHERE job_id=?1 AND message_id=?2",
                    params![job_id, segment.message_id],
                    |row| row.get(0),
                )?;
                if state == "deferred" {
                    deferred += tx.execute(
                        "DELETE FROM segment WHERE id=?1 AND job_id=?2 AND state='pending'",
                        params![id, job_id],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(deferred)
    }

    /// Mark selected candidates requested and append their missing segments
    /// atomically. Retrying this operation never resets existing segment state.
    pub fn activate_recovery_candidates(&self, job_id: i64, ids: &[i64]) -> Result<usize, Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let mut appended = 0;
        for id in ids {
            let segment = tx
                .query_row(
                    "SELECT filename,part,message_id,begin,end,article FROM recovery_candidate
                 WHERE id=?1 AND job_id=?2 AND state='deferred'",
                    params![id, job_id],
                    new_segment_from_row,
                )
                .optional()?;
            if let Some(segment) = segment {
                appended += append_segment(&tx, job_id, &segment)?;
                tx.execute("UPDATE recovery_candidate SET state='requested' WHERE id=?1 AND job_id=?2 AND state='deferred'", params![id, job_id])?;
            }
        }
        tx.commit()?;
        Ok(appended)
    }

    pub fn exhaust_recovery_candidates(&self, job_id: i64, ids: &[i64]) -> Result<(), Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        for id in ids {
            tx.execute(
                "UPDATE recovery_candidate SET state='exhausted' WHERE id=?1 AND job_id=?2",
                params![id, job_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

pub(in crate::catalog) fn add_candidate(
    tx: &Transaction<'_>,
    job_id: i64,
    segment: &NewSegment,
    blocks: Option<i64>,
    parts: i64,
) -> Result<usize, Error> {
    validate_segment(segment)?;
    if parts < segment.part || blocks.is_some_and(|blocks| blocks <= 0) {
        return Err(Error::Header(
            "invalid recovery candidate part/block count".into(),
        ));
    }
    let inserted=tx.execute(
        "INSERT INTO recovery_candidate(job_id,filename,part,message_id,begin,end,article,recovery_blocks,expected_parts,state)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'deferred') ON CONFLICT(job_id,message_id) DO NOTHING",
        params![job_id,segment.filename,segment.part,segment.message_id,segment.begin,segment.end,segment.article,blocks,parts])?;
    if inserted == 0 {
        // Later headers may reveal a larger true total than a legacy cache's
        // highest known part. Keep attempts durable while improving this bound.
        tx.execute(
            "UPDATE recovery_candidate SET expected_parts=MAX(expected_parts,?1)
                    WHERE job_id=?2 AND message_id=?3 AND filename=?4 AND part=?5",
            params![
                parts,
                job_id,
                segment.message_id,
                segment.filename,
                segment.part
            ],
        )?;
    }
    Ok(inserted)
}

fn new_segment_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NewSegment> {
    Ok(NewSegment {
        filename: row.get(0)?,
        part: row.get(1)?,
        message_id: row.get(2)?,
        begin: row.get(3)?,
        end: row.get(4)?,
        article: row.get(5)?,
    })
}

fn candidate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecoveryCandidate> {
    Ok(RecoveryCandidate {
        id: row.get(0)?,
        job_id: row.get(1)?,
        segment: NewSegment {
            filename: row.get(2)?,
            part: row.get(3)?,
            message_id: row.get(4)?,
            begin: row.get(5)?,
            end: row.get(6)?,
            article: row.get(7)?,
        },
        recovery_blocks: row.get(8)?,
        expected_parts: row.get(9)?,
        state: row.get(10)?,
    })
}
