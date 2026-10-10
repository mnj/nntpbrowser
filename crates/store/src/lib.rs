//! Local storage. Overview volume lives in OSF1 files. Servers, grouped
//! posts, and the download queue live in SQLite.

mod catalog;
mod dateutil;
mod error;
mod grouper;
mod osf;

pub use catalog::{
    recovery_volume_blocks, recovery_volume_range, Catalog, FileRow, GroupRow, IncomingFile,
    IncomingPost, JobRecoveryContext, JobRow, NewSegment, NewServer, PostRow, RecoveryCandidate,
    SegmentRow, ServerRow, ThumbRow,
};
pub use dateutil::parse_overview_date;
pub use error::Error;
pub use grouper::{classify, parse_subject, parse_subject_legacy, FileKind, ParsedSegment};
pub use osf::{group_slug, BadSpan, HeaderStore, Record};
