//! Reproducible synthetic overview-storage comparison. Run with:
//! cargo run -p store --example storage_bench --release -- [record_count]
//!
//! This compares OSF1 plus grouped SQLite metadata with one SQLite row per
//! header. Binary multipart subjects have repeating structure and message IDs
//! contain varied pseudo-random bits. Results depend on workload and disk;
//! scans run with a warm filesystem cache. Generated data is kept in a unique
//! temporary directory printed at the end so sizes can be inspected.

use rusqlite::{params, Connection};
use std::{fs, time::Instant};
use store::{parse_subject, Catalog, HeaderStore, IncomingFile, IncomingPost, NewServer, Record};
fn dir_bytes(path: &std::path::Path) -> u64 {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dir_bytes(&path)
            } else {
                fs::metadata(path).unwrap().len()
            }
        })
        .sum()
}
fn main() {
    let count = std::env::args()
        .nth(1)
        .map(|value| {
            value
                .parse::<u64>()
                .expect("record_count must be an integer")
        })
        .unwrap_or(200_000);
    assert!(count > 0, "record_count must be greater than zero");
    let root = std::env::temp_dir().join(format!(
        "nntpbrowser-storage-bench-data-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let records: Vec<_> = (1..=count)
        .map(|article| Record {
            article,
            date_unix: 1_700_000_000 + article as i64 / 200,
            date_raw: None,
            bytes: 700_000 + article % 1000,
            lines: 9000,
            message_id: format!(
                "<{:016x}.{:016x}@posting.example.net>",
                article.wrapping_mul(0x9e3779b97f4a7c15),
                article.wrapping_mul(0x94d049bb133111eb)
            ),
            subject: format!(
                "Article number {:06} [01/10] - \"file-{:06}.part01.rar\" yEnc ({}/200)",
                article / 200,
                article / 200,
                article % 200 + 1
            ),
            from: format!(
                "Poster {} <poster{}@example.net>",
                article / 200 % 31,
                article / 200 % 31
            ),
        })
        .collect();
    let logical: usize = records
        .iter()
        .map(|r| 41 + r.message_id.len() + r.subject.len() + r.from.len())
        .sum();
    let store = HeaderStore::open(root.join("osf")).unwrap();
    let t = Instant::now();
    for batch in records.chunks(2000) {
        store.append_range(1, "alt.test", batch).unwrap();
    }
    let osf_write = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let mut read = 0;
    assert!(store
        .for_each(1, "alt.test", |record| {
            assert_eq!(record, records[read]);
            read += 1;
        })
        .unwrap()
        .is_empty());
    assert_eq!(read, records.len());
    let osf_scan = t.elapsed().as_secs_f64();
    let t = Instant::now();
    store.compact_if_needed(1, "alt.test").unwrap();
    let compact = t.elapsed().as_secs_f64();
    let osf_size = dir_bytes(&root.join("osf"));
    let mut db = Connection::open(root.join("overview.sqlite")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE overview(article INTEGER PRIMARY KEY, date_unix INTEGER NOT NULL, bytes INTEGER NOT NULL, lines INTEGER NOT NULL, message_id TEXT NOT NULL, subject TEXT NOT NULL, poster TEXT NOT NULL); CREATE UNIQUE INDEX overview_message ON overview(message_id);").unwrap();
    let t = Instant::now();
    for batch in records.chunks(2000) {
        let tx = db.transaction().unwrap();
        {
            let mut stmt = tx
                .prepare_cached("INSERT INTO overview VALUES (?1,?2,?3,?4,?5,?6,?7)")
                .unwrap();
            for r in batch {
                stmt.execute(params![
                    r.article,
                    r.date_unix,
                    r.bytes,
                    r.lines,
                    r.message_id,
                    r.subject,
                    r.from
                ])
                .unwrap();
            }
        }
        tx.commit().unwrap();
    }
    let sql_write = t.elapsed().as_secs_f64();
    db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    let sql_size = fs::metadata(root.join("overview.sqlite")).unwrap().len();
    let t = Instant::now();
    let mut count = 0;
    let mut stmt=db.prepare("SELECT article,date_unix,bytes,lines,message_id,subject,poster FROM overview ORDER BY article").unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok(Record {
                article: row.get(0)?,
                date_unix: row.get(1)?,
                bytes: row.get(2)?,
                lines: row.get(3)?,
                message_id: row.get(4)?,
                subject: row.get(5)?,
                from: row.get(6)?,
                date_raw: None,
            })
        })
        .unwrap();
    for row in rows {
        assert_eq!(row.unwrap(), records[count]);
        count += 1;
    }
    assert_eq!(count, records.len());
    let sql_scan = t.elapsed().as_secs_f64();
    drop(stmt);
    db.execute_batch("DROP INDEX overview_message; VACUUM; PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();
    let unindexed_size = fs::metadata(root.join("overview.sqlite")).unwrap().len();
    let catalog_path = root.join("catalog.sqlite");
    let catalog = Catalog::open(&catalog_path).unwrap();
    let sid = catalog
        .insert_server(&NewServer {
            name: "test".into(),
            host: "localhost".into(),
            port: 119,
            tls: false,
            starttls: false,
            username: None,
            password: None,
            priority: 1,
            max_connections: 1,
            pipeline_depth: 1,
            fill: false,
        })
        .unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    let mut posts: std::collections::HashMap<String, IncomingPost> =
        std::collections::HashMap::new();
    for record in &records {
        let parsed = parse_subject(&record.subject, &record.from);
        let post = posts
            .entry(parsed.group_key.clone())
            .or_insert_with(|| IncomingPost {
                group_key: parsed.group_key,
                title: parsed.title,
                poster: record.from.clone(),
                posted_at: record.date_unix,
                files: vec![IncomingFile {
                    name: parsed.filename.unwrap(),
                    size: 0,
                    expected_segments: parsed.total.unwrap_or(1) as i64,
                    kind: parsed.kind,
                }],
            });
        post.files[0].size += record.bytes as i64;
    }
    let grouped_count = posts.len();
    let t = Instant::now();
    catalog
        .merge_posts(gid, &posts.into_values().collect::<Vec<_>>())
        .unwrap();
    let catalog_write = t.elapsed().as_secs_f64();
    drop(catalog);
    let catalog_size = fs::metadata(catalog_path).unwrap().len();
    println!("sqlite_without_message_index_bytes={} hybrid_catalog_bytes={} grouped_posts={} catalog_write_s={:.4}", unindexed_size,catalog_size,grouped_count,catalog_write);
    println!("rows={} logical_bytes={} osf_bytes={} sqlite_bytes={} osf_write_s={:.4} sqlite_write_s={:.4} osf_scan_s={:.4} sqlite_scan_s={:.4} compact_s={:.4} data={}", records.len(),logical,osf_size,sql_size,osf_write,sql_write,osf_scan,sql_scan,compact,root.display());
}
