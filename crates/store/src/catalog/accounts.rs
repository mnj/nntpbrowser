use super::*;

impl Catalog {
    pub fn insert_server(&self, server: &NewServer) -> Result<i64, Error> {
        self.insert_server_enabled(server, true)
    }

    /// Persist the initial enabled state in the same statement as the server.
    pub fn insert_server_enabled(&self, server: &NewServer, enabled: bool) -> Result<i64, Error> {
        validate_tls_modes(server)?;
        let conn = self.lock();
        conn.execute(
            "INSERT INTO server (name, host, port, tls, username, password, priority, max_connections, pipeline_depth, fill, enabled, header_compression, starttls)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'unknown', ?12)",
            params![
                server.name,
                server.host,
                server.port,
                server.tls as i64,
                server.username,
                server.password,
                server.priority,
                server.max_connections,
                server.pipeline_depth,
                server.fill as i64,
                enabled as i64,
                server.starttls as i64,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_server(&self, id: i64, server: &NewServer, enabled: bool) -> Result<(), Error> {
        validate_tls_modes(server)?;
        let conn = self.lock();
        conn.execute(
            "UPDATE server SET name=?1, host=?2, port=?3, tls=?4, username=?5, password=?6, priority=?7, max_connections=?8, pipeline_depth=?9, fill=?10, enabled=?11, starttls=?13 WHERE id=?12",
            params![
                server.name,
                server.host,
                server.port,
                server.tls as i64,
                server.username,
                server.password,
                server.priority,
                server.max_connections,
                server.pipeline_depth,
                server.fill as i64,
                enabled as i64,
                id,
                server.starttls as i64,
            ],
        )?;
        Ok(())
    }

    pub fn delete_server(&self, id: i64) -> Result<(), Error> {
        self.lock()
            .execute("DELETE FROM server WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn list_servers(&self) -> Result<Vec<ServerRow>, Error> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, host, port, tls, username, password, priority, max_connections, pipeline_depth, fill, enabled, header_compression, starttls
             FROM server ORDER BY priority, id",
        )?;
        let rows = stmt.query_map([], server_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    pub fn server_by_id(&self, id: i64) -> Result<Option<ServerRow>, Error> {
        self.lock().query_row(
            "SELECT id, name, host, port, tls, username, password, priority, max_connections, pipeline_depth, fill, enabled, header_compression, starttls FROM server WHERE id=?1",
            [id], server_from_row,
        ).optional().map_err(Error::from)
    }

    pub fn set_header_compression(&self, id: i64, strategy: &str) -> Result<(), Error> {
        self.lock().execute(
            "UPDATE server SET header_compression=?1 WHERE id=?2",
            params![strategy, id],
        )?;
        Ok(())
    }

    pub fn subscribe(&self, server_id: i64, name: &str) -> Result<i64, Error> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO group_sub (server_id, name, low, high, watermark, subscribed)
             VALUES (?1, ?2, 0, 0, 0, 1)
             ON CONFLICT(server_id, name) DO UPDATE SET subscribed=1",
            params![server_id, name],
        )?;
        let id = conn.query_row(
            "SELECT id FROM group_sub WHERE server_id=?1 AND name=?2",
            params![server_id, name],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn list_groups(&self, server_id: i64) -> Result<Vec<GroupRow>, Error> {
        let conn = self.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT id, server_id, name, low, high, watermark, subscribed, last_refresh
             FROM group_sub WHERE server_id=?1 ORDER BY name",
        )?;
        let rows = stmt.query_map([server_id], group_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    pub fn group_by_id(&self, id: i64) -> Result<Option<GroupRow>, Error> {
        self.lock().query_row(
            "SELECT id, server_id, name, low, high, watermark, subscribed, last_refresh FROM group_sub WHERE id=?1",
            [id], group_from_row,
        ).optional().map_err(Error::from)
    }

    pub fn set_group_marks(
        &self,
        group_id: i64,
        low: i64,
        high: i64,
        watermark: i64,
        last_refresh: Option<i64>,
    ) -> Result<(), Error> {
        self.lock().execute(
            "UPDATE group_sub SET low=?1, high=?2, watermark=?3, last_refresh=?4 WHERE id=?5",
            params![low, high, watermark, last_refresh, group_id],
        )?;
        Ok(())
    }
}

fn server_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ServerRow> {
    Ok(ServerRow {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        port: row.get(3)?,
        tls: row.get::<_, i64>(4)? != 0,
        starttls: row.get::<_, i64>(13)? != 0,
        username: row.get(5)?,
        password: row.get(6)?,
        priority: row.get(7)?,
        max_connections: row.get(8)?,
        pipeline_depth: row.get(9)?,
        fill: row.get::<_, i64>(10)? != 0,
        enabled: row.get::<_, i64>(11)? != 0,
        header_compression: row.get(12)?,
    })
}

fn validate_tls_modes(server: &NewServer) -> Result<(), Error> {
    if server.tls && server.starttls {
        return Err(Error::Header(
            "implicit TLS and STARTTLS are mutually exclusive".into(),
        ));
    }
    Ok(())
}

fn group_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GroupRow> {
    Ok(GroupRow {
        id: row.get(0)?,
        server_id: row.get(1)?,
        name: row.get(2)?,
        low: row.get(3)?,
        high: row.get(4)?,
        watermark: row.get(5)?,
        subscribed: row.get::<_, i64>(6)? != 0,
        last_refresh: row.get(7)?,
    })
}
