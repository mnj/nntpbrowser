use super::*;

impl Catalog {
    /// Remove only this article's cached thumbnail metadata.
    pub fn remove_thumb_for_post(&self, post_id: i64) -> Result<bool, Error> {
        Ok(self
            .lock()
            .execute("DELETE FROM thumb WHERE post_id=?1", [post_id])?
            > 0)
    }

    pub fn insert_thumb(
        &self,
        post_id: i64,
        path: &Path,
        width: i64,
        height: i64,
    ) -> Result<(), Error> {
        self.lock().execute(
            "INSERT INTO thumb (post_id, path, width, height) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(post_id) DO UPDATE SET path=excluded.path, width=excluded.width, height=excluded.height",
            params![post_id, path.display().to_string(), width, height],
        )?;
        Ok(())
    }

    pub fn thumb_for_post(&self, post_id: i64) -> Result<Option<ThumbRow>, Error> {
        let row = self
            .lock()
            .query_row(
                "SELECT post_id, path, width, height FROM thumb WHERE post_id=?1",
                [post_id],
                |row| {
                    Ok(ThumbRow {
                        post_id: row.get(0)?,
                        path: row.get(1)?,
                        width: row.get(2)?,
                        height: row.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }
}
