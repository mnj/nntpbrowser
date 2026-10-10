use super::*;

impl Catalog {
    /// Delete a subscription and its article/file/thumbnail metadata. Download
    /// history and segments remain; the job foreign key detaches their article.
    /// The caller must remove generated caches before deleting the row.
    pub fn unsubscribe(&self, server_id: i64, name: &str) -> Result<(), Error> {
        self.lock().execute(
            "DELETE FROM group_sub WHERE server_id=?1 AND name=?2",
            params![server_id, name],
        )?;
        Ok(())
    }

    pub fn post_ids_for_group(&self, group_id: i64) -> Result<Vec<i64>, Error> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached("SELECT id FROM post WHERE group_id=?1 ORDER BY id")?;
        let rows = stmt.query_map([group_id], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }
}
