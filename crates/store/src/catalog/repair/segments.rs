use super::*;

impl Catalog {
    /// Add discovered data segments without replacing any active/written part.
    /// A different message ID may replace a failed part, never a written one.
    pub fn append_job_segments(
        &self,
        job_id: i64,
        segments: &[NewSegment],
    ) -> Result<usize, Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let mut appended = 0;
        for segment in segments {
            appended += append_segment(&tx, job_id, segment)?;
        }
        tx.commit()?;
        Ok(appended)
    }
}

pub(in crate::catalog) fn validate_segment(segment: &NewSegment) -> Result<(), Error> {
    if !super::queue::valid_filename(&segment.filename)
        || segment.part <= 0
        || segment.begin < 0
        || segment.end < segment.begin
        || segment.article.is_some_and(|number| number < 0)
        || segment.message_id.is_empty()
        || segment.message_id.len() > 497
        || segment
            .message_id
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(Error::Header(
            "invalid recovery/download segment identity or bounds".into(),
        ));
    }
    Ok(())
}

pub(super) fn append_segment(
    tx: &Transaction<'_>,
    job_id: i64,
    segment: &NewSegment,
) -> Result<usize, Error> {
    validate_segment(segment)?;
    let already_known: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM segment WHERE job_id=?1 AND message_id=?4)
         OR EXISTS(SELECT 1 FROM segment WHERE job_id=?1 AND filename=?2 AND part=?3 AND state NOT IN ('failed','missing'))",
        params![job_id,segment.filename,segment.part,segment.message_id], |row| row.get(0))?;
    if already_known {
        return Ok(0);
    }
    tx.execute("DELETE FROM segment WHERE job_id=?1 AND filename=?2 AND part=?3 AND state IN ('failed','missing')",
        params![job_id,segment.filename,segment.part])?;
    let inserted = tx.execute(
        "INSERT INTO segment(job_id,filename,part,message_id,begin,end,article,state,server_id)
         VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',NULL)",
        params![
            job_id,
            segment.filename,
            segment.part,
            segment.message_id,
            segment.begin,
            segment.end,
            segment.article
        ],
    )?;
    Ok(inserted)
}
