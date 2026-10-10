use super::*;

impl Catalog {
    pub fn recovery_context(&self, job_id: i64) -> Result<Option<JobRecoveryContext>, Error> {
        let conn = self.lock();
        let context = conn.query_row(
            "SELECT server_id,group_id,group_name,poster,group_key,set_id,discovery_exhausted,posted_at
             FROM job_recovery WHERE job_id=?1", [job_id], |row| Ok(JobRecoveryContext {
                server_id: row.get(0)?, group_id: row.get(1)?, group: row.get(2)?,
                poster: row.get(3)?, group_key: row.get(4)?, set_id: row.get(5)?,
                discovery_exhausted: row.get(6)?, posted_at:row.get(7)?, discovery_ranges: Vec::new(),
            })).optional()?;
        context
            .map(|mut context| {
                context.discovery_ranges = read_ranges(&conn, job_id)?;
                Ok(context)
            })
            .transpose()
    }

    /// Source identity is immutable once registered. Progress only advances:
    /// committed scan spans/exhaustion cannot be erased by a stale snapshot.
    pub fn set_recovery_context(
        &self,
        job_id: i64,
        context: &JobRecoveryContext,
    ) -> Result<(), Error> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        save_context(&tx, job_id, context)?;
        tx.commit()?;
        Ok(())
    }
}

pub(in crate::catalog) fn save_context(
    tx: &Transaction<'_>,
    job_id: i64,
    context: &JobRecoveryContext,
) -> Result<(), Error> {
    if context.set_id.as_ref().is_some_and(|id| {
        id.len() != 32
            || !id
                .bytes()
                .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
    }) || context
        .group
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(Error::Header("invalid recovery source/set identity".into()));
    }
    let old: Option<(String, String, String, Option<String>)> = tx
        .query_row(
            "SELECT group_name,poster,group_key,set_id FROM job_recovery WHERE job_id=?1",
            [job_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    if let Some((group, poster, key, set)) = old {
        if group != context.group
            || poster != context.poster
            || key != context.group_key
            || set
                .as_ref()
                .zip(context.set_id.as_ref())
                .is_some_and(|(old, new)| old != new)
        {
            return Err(Error::Header(
                "recovery source or pinned PAR2 set cannot change".into(),
            ));
        }
    } else if let Some(group_id) = context.group_id {
        let source: Option<(i64, String)> = tx
            .query_row(
                "SELECT server_id,name FROM group_sub WHERE id=?1",
                [group_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if source
            .is_none_or(|(server, name)| Some(server) != context.server_id || name != context.group)
        {
            return Err(Error::Header(
                "recovery group does not match its server".into(),
            ));
        }
    }
    tx.execute(
        "INSERT INTO job_recovery(job_id,server_id,group_id,group_name,poster,group_key,set_id,discovery_exhausted,posted_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(job_id) DO UPDATE SET set_id=COALESCE(job_recovery.set_id,excluded.set_id),
         discovery_exhausted=MAX(job_recovery.discovery_exhausted,excluded.discovery_exhausted)",
        params![job_id,context.server_id,context.group_id,context.group,context.poster,context.group_key,context.set_id,context.discovery_exhausted,context.posted_at])?;
    for &(first, last) in &context.discovery_ranges {
        merge_range(tx, job_id, first, last)?;
    }
    Ok(())
}
