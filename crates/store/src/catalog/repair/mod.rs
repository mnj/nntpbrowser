//! Durable, on-demand PAR2 candidates. The engine verifies contents; these
//! transactions preserve queue identities and successful discovery progress.
use super::*;
use rusqlite::Transaction;

mod candidates;
mod context;
mod ranges;
mod segments;

pub(super) use candidates::add_candidate;
pub(super) use context::save_context;
use ranges::{merge_range, read_ranges};
use segments::append_segment;
pub(super) use segments::validate_segment;
