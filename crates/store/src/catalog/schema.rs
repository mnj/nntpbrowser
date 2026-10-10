use super::*;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE server (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL,
    tls INTEGER NOT NULL,
    username TEXT,
    password TEXT,
    priority INTEGER NOT NULL,
    max_connections INTEGER NOT NULL,
    pipeline_depth INTEGER NOT NULL,
    fill INTEGER NOT NULL,
    enabled INTEGER NOT NULL,
    header_compression TEXT NOT NULL DEFAULT 'unknown',
    starttls INTEGER NOT NULL DEFAULT 0 CHECK(starttls IN (0,1))
);
CREATE TABLE group_sub (
    id INTEGER PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES server(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    low INTEGER NOT NULL,
    high INTEGER NOT NULL,
    watermark INTEGER NOT NULL,
    subscribed INTEGER NOT NULL,
    last_refresh INTEGER,
    UNIQUE(server_id, name)
);
CREATE TABLE post (
    id INTEGER PRIMARY KEY,
    group_id INTEGER NOT NULL REFERENCES group_sub(id) ON DELETE CASCADE,
    group_key TEXT NOT NULL,
    title TEXT NOT NULL,
    poster TEXT NOT NULL,
    posted_at INTEGER NOT NULL,
    total_bytes INTEGER NOT NULL,
    file_count INTEGER NOT NULL,
    segment_count INTEGER NOT NULL,
    flags INTEGER NOT NULL,
    UNIQUE(group_id, group_key)
);
CREATE TABLE file (
    id INTEGER PRIMARY KEY,
    post_id INTEGER NOT NULL REFERENCES post(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    size INTEGER NOT NULL,
    expected_segments INTEGER NOT NULL,
    kind TEXT NOT NULL,
    UNIQUE(post_id, name)
);
CREATE TABLE job (
    id INTEGER PRIMARY KEY,
    post_id INTEGER REFERENCES post(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    dest_dir TEXT NOT NULL,
    dest_path TEXT NOT NULL,
    state TEXT NOT NULL,
    bytes_done INTEGER NOT NULL,
    bytes_total INTEGER NOT NULL,
    error TEXT,
    password TEXT
);
CREATE TABLE segment (
    id INTEGER PRIMARY KEY,
    job_id INTEGER NOT NULL REFERENCES job(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    part INTEGER NOT NULL,
    message_id TEXT NOT NULL,
    begin INTEGER NOT NULL,
    end INTEGER NOT NULL,
    article INTEGER,
    state TEXT NOT NULL,
    server_id INTEGER,
    UNIQUE(job_id, message_id)
);
CREATE TABLE thumb (
    id INTEGER PRIMARY KEY,
    post_id INTEGER NOT NULL REFERENCES post(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    UNIQUE(post_id)
);
CREATE INDEX post_group_posted ON post(group_id, posted_at DESC, id DESC);
CREATE INDEX file_post ON file(post_id);
CREATE INDEX segment_job ON segment(job_id);
"#;

const REPAIR_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS job_recovery (
    job_id INTEGER PRIMARY KEY REFERENCES job(id) ON DELETE CASCADE,
    server_id INTEGER REFERENCES server(id) ON DELETE SET NULL,
    group_id INTEGER REFERENCES group_sub(id) ON DELETE SET NULL,
    group_name TEXT NOT NULL,
    poster TEXT NOT NULL,
    group_key TEXT NOT NULL,
    posted_at INTEGER,
    set_id TEXT,
    discovery_exhausted INTEGER NOT NULL CHECK(discovery_exhausted IN (0,1))
);
CREATE INDEX IF NOT EXISTS job_recovery_group ON job_recovery(group_id);
CREATE INDEX IF NOT EXISTS job_recovery_server ON job_recovery(server_id);
CREATE TABLE IF NOT EXISTS recovery_candidate (
    id INTEGER PRIMARY KEY,
    job_id INTEGER NOT NULL REFERENCES job(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    part INTEGER NOT NULL CHECK(part > 0),
    message_id TEXT NOT NULL,
    begin INTEGER NOT NULL CHECK(begin >= 0),
    end INTEGER NOT NULL CHECK(end >= begin),
    article INTEGER CHECK(article >= 0),
    recovery_blocks INTEGER CHECK(recovery_blocks > 0),
    expected_parts INTEGER NOT NULL CHECK(expected_parts >= part),
    state TEXT NOT NULL CHECK(state IN ('deferred','requested','exhausted')),
    UNIQUE(job_id,message_id)
);
CREATE TABLE IF NOT EXISTS recovery_scan (
    job_id INTEGER NOT NULL REFERENCES job(id) ON DELETE CASCADE,
    first INTEGER NOT NULL CHECK(first >= 0),
    last INTEGER NOT NULL CHECK(last >= first),
    PRIMARY KEY(job_id,first)
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS recovery_group_deleted BEFORE DELETE ON group_sub
BEGIN
    UPDATE job_recovery SET group_id=NULL, discovery_exhausted=1 WHERE group_id=OLD.id;
END;
CREATE TRIGGER IF NOT EXISTS recovery_server_deleted BEFORE DELETE ON server
BEGIN
    UPDATE job_recovery SET server_id=NULL, discovery_exhausted=1 WHERE server_id=OLD.id;
END;
"#;

pub(super) fn prepare(conn: &Connection) -> Result<(), Error> {
    let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(Error::Header(format!("journal mode is {mode}")));
    }
    conn.execute("PRAGMA foreign_keys=ON", [])?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version == 0 {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(SCHEMA)?;
        tx.execute_batch(REPAIR_SCHEMA)?;
        tx.execute("PRAGMA user_version=3", [])?;
        tx.commit()?;
    } else if version == 1 {
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "ALTER TABLE server ADD COLUMN starttls INTEGER NOT NULL DEFAULT 0 CHECK(starttls IN (0,1))",
            [],
        )?;
        tx.execute_batch(REPAIR_SCHEMA)?;
        tx.execute("PRAGMA user_version=3", [])?;
        tx.commit()?;
    } else if version == 2 {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(REPAIR_SCHEMA)?;
        tx.execute("PRAGMA user_version=3", [])?;
        tx.commit()?;
    } else if version != 3 {
        return Err(Error::Header(format!(
            "unsupported catalog version {version}"
        )));
    }
    // A crash can leave an in-flight segment with no worker owning it. Reset
    // transient states atomically so resumed jobs cannot skip unwritten parts.
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE segment SET state='pending' WHERE state='fetching'",
        [],
    )?;
    tx.execute(
        "UPDATE job SET state='queued' WHERE state IN ('fetching','decoding','repair','extract','recovery_headers','recovery_fetching')",
        [],
    )?;
    tx.commit()?;
    Ok(())
}
