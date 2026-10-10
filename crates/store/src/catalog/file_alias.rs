//! Correct the old quoted-space parser's file alias without changing post IDs.

use super::IncomingPost;

pub(super) fn legacy_alias(post: &IncomingPost, name: &str) -> Option<String> {
    if !name.contains(' ') {
        return None;
    }
    let legacy = crate::parse_subject_legacy(&format!("\"{name}\""), &post.poster);
    let alias = legacy.filename?;
    if alias == name {
        return None;
    }
    let (prefix, poster) = legacy.group_key.rsplit_once('\n')?;
    let (title, existing_poster) = post.group_key.rsplit_once('\n')?;
    // Old keys retain the opening quote and all words before the final filename
    // token. That exact suffix proves this is the parser's truncation, rather
    // than a distinct short-named file in an otherwise normally grouped post.
    (prefix.starts_with('"') && title.ends_with(prefix) && existing_poster == poster)
        .then_some(alias)
}
