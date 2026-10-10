use super::*;

fn segment(name: &str, part: i64, id: &str) -> NewSegment {
    NewSegment {
        filename: name.into(),
        part,
        message_id: id.into(),
        begin: 0,
        end: 10,
        article: Some(100 + part),
    }
}

fn context() -> JobRecoveryContext {
    JobRecoveryContext {
        server_id: None,
        group_id: None,
        group: String::new(),
        poster: String::new(),
        group_key: String::new(),
        posted_at: None,
        set_id: None,
        discovery_exhausted: false,
        discovery_ranges: Vec::new(),
    }
}

fn job(catalog: &Catalog, segments: &[NewSegment]) -> i64 {
    catalog
        .create_job(
            None,
            "Article",
            Path::new("downloads/article"),
            None,
            segments,
        )
        .unwrap()
}

mod candidates;
mod context;
mod migration;
mod ranges;

mod validation;
