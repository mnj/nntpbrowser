use super::*;
use store::{IncomingFile, IncomingPost, Record};

const INDEX: &[u8] = include_bytes!("fixtures/par2/payload.par2");
const VOLUME: &[u8] = include_bytes!("fixtures/par2/payload.vol0+1.par2");
const DATA: &[u8] = b"ABCDEFGH";

fn local_recovery(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("payload.par2"), INDEX).unwrap();
    std::fs::write(dir.join("payload.vol0+1.par2"), VOLUME).unwrap();
}

async fn drive_until_terminal(worker: &mut Worker, id: i64) -> store::JobRow {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        worker.drive_one().await;
        let job = worker.catalog.job_by_id(id).unwrap().unwrap();
        if matches!(job.state.as_str(), "done" | "failed") {
            return job;
        }
        assert!(std::time::Instant::now() < deadline, "{job:?}");
    }
}

fn cached_first_header(worker: &Worker, sid: i64) -> i64 {
    let gid = worker.catalog.subscribe(sid, "alt.test").unwrap();
    let subject = "Set [01/01] - \"payload.bin\" yEnc (1/2)";
    worker
        .headers
        .append_range(
            sid,
            "alt.test",
            &[Record {
                article: 10,
                date_unix: 1,
                date_raw: None,
                bytes: 4,
                lines: 1,
                message_id: "payload1@test".into(),
                subject: subject.into(),
                from: "Poster".into(),
            }],
        )
        .unwrap();
    worker
        .catalog
        .upsert_posts(
            gid,
            &[IncomingPost {
                group_key: store::parse_subject(subject, "Poster").group_key,
                title: "Set".into(),
                poster: "Poster".into(),
                posted_at: 1,
                files: vec![IncomingFile {
                    name: "payload.bin".into(),
                    size: 4,
                    expected_segments: 2,
                    kind: store::FileKind::Data,
                }],
            }],
        )
        .unwrap();
    worker.catalog.page_posts(gid, None, 1, None).unwrap()[0].id
}

mod auxiliary;
mod headers;
mod recovery;
mod recovery_controls;

mod automatic;

mod volumes;
