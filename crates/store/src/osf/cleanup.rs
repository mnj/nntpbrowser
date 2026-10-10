use super::*;

impl HeaderStore {
    /// Remove only this group's cache, including pending journals. Refuse
    /// symlinked ancestors instead of following them outside the cache root.
    pub fn remove_group(&self, server_id: i64, group: &str) -> Result<(), Error> {
        if !self.check_group_removal(server_id, group)? {
            return Ok(());
        }
        fs::remove_dir_all(
            self.root
                .join(server_id.to_string())
                .join(group_slug(group)),
        )?;
        Ok(())
    }

    pub fn check_group_removal(&self, server_id: i64, group: &str) -> Result<bool, Error> {
        let server = self.root.join(server_id.to_string());
        let dir = server.join(group_slug(group));
        for path in [&self.root, &server, &dir] {
            match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(Error::Header(format!(
                        "unsafe cache directory: {}",
                        path.display()
                    )))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(true)
    }
}
