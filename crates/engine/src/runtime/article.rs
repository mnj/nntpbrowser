//! Selection downloads the article body without implicitly queueing attachments.

use super::{Event, Worker};
use nntp::ArticleRef;

mod cache;

const ARTICLE_CAP: usize = 4 * 1024 * 1024;

impl Worker {
    pub(super) async fn read_article(&mut self, post_id: i64) {
        match self.article_body(post_id).await {
            Ok(body) => {
                let binary = body.split(|byte| *byte == b'\n').any(|line| {
                    line.starts_with(b"=ybegin ")
                        || line.starts_with(b"begin-base64 ")
                        || (line.starts_with(b"begin ")
                            && line.get(6..9).is_some_and(|mode| {
                                mode.iter().all(|byte| (b'0'..=b'7').contains(byte))
                            }))
                });
                let text = if binary {
                    "This article contains a binary attachment. Its files are listed above; eligible images and videos can be previewed in memory in Gallery.".into()
                } else {
                    String::from_utf8_lossy(&body).into_owned()
                };
                let _ = self.evt.send(Event::ArticleContent {
                    post_id,
                    text,
                    binary,
                });
            }
            Err(error) => {
                let _ = self.evt.send(Event::ArticleReadFailed { post_id, error });
            }
        }
    }

    async fn article_body(&mut self, post_id: i64) -> Result<Vec<u8>, String> {
        let Some((server, group, post)) = self.find_post(post_id) else {
            return Err("article was not found".into());
        };
        let segments = self.segments_for_key(server.id, &group.name, &post.group_key)?;
        let first = segments
            .iter()
            .min_by_key(|segment| segment.article.unwrap_or(i64::MAX))
            .ok_or("article headers were not found")?;
        if let Some(body) = self.cached_article_body(post_id, &first.message_id, ARTICLE_CAP) {
            return Ok(body);
        }
        let (_, body) = self
            .body_from_farm_limited(
                &ArticleRef::MessageId(first.message_id.clone()),
                ARTICLE_CAP,
            )
            .await
            .map_err(|err| err.to_string())?;
        self.store_article_body(post_id, &first.message_id, &body)?;
        Ok(body)
    }
}
