use super::*;

impl Catalog {
    pub fn recovery_scanned_ranges(&self, job_id: i64) -> Result<Vec<(u64, u64)>, Error> {
        read_ranges(&self.lock(), job_id)
    }

    /// Record only a successfully published OVER window, including empty ones.
    /// Adjacent/overlapping inclusive spans merge in the same transaction.
    pub fn mark_recovery_scanned(&self, job_id: i64, first: u64, last: u64) -> Result<(), Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        merge_range(&tx, job_id, first, last)?;
        tx.commit()?;
        Ok(())
    }
}

pub(super) fn read_ranges(conn: &Connection, job_id: i64) -> Result<Vec<(u64, u64)>, Error> {
    let mut statement =
        conn.prepare_cached("SELECT first,last FROM recovery_scan WHERE job_id=?1 ORDER BY first")?;
    let rows = statement.query_map([job_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
}

pub(super) fn merge_range(
    tx: &Transaction<'_>,
    job_id: i64,
    first: u64,
    last: u64,
) -> Result<(), Error> {
    let mut first = i64::try_from(first)
        .map_err(|_| Error::Header("recovery article number exceeds catalog range".into()))?;
    let mut last = i64::try_from(last)
        .map_err(|_| Error::Header("recovery article number exceeds catalog range".into()))?;
    if first > last {
        return Err(Error::Header("invalid recovery discovery range".into()));
    }
    let adjacent: (Option<i64>, Option<i64>) = tx.query_row(
        "SELECT MIN(first),MAX(last) FROM recovery_scan WHERE job_id=?1 AND first<=?2 AND last>=?3",
        params![job_id, last.saturating_add(1), first.saturating_sub(1)],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    first = adjacent.0.map_or(first, |old| old.min(first));
    last = adjacent.1.map_or(last, |old| old.max(last));
    tx.execute(
        "DELETE FROM recovery_scan WHERE job_id=?1 AND first<=?2 AND last>=?3",
        params![job_id, last.saturating_add(1), first.saturating_sub(1)],
    )?;
    tx.execute(
        "INSERT INTO recovery_scan VALUES(?1,?2,?3)",
        params![job_id, first, last],
    )?;
    Ok(())
}
