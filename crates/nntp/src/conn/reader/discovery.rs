//! New groups/articles and standard extensible LIST queries.
use super::*;

impl Connection {
    pub async fn help(&mut self) -> Result<String, Error> {
        let lines = self.reader_data("HELP\r\n", 100).await?.1;
        Ok(String::from_utf8_lossy(&join_lines(&lines)).into_owned())
    }

    pub async fn new_groups(&mut self, since: &NewsDateTime) -> Result<Vec<GroupInfo>, Error> {
        let (_, lines) = self
            .reader_data(&format!("NEWGROUPS {}\r\n", since.wire()?), 231)
            .await?;
        lines.iter().map(|line| parse_group_info(line)).collect()
    }

    /// Message IDs are returned in wire form; an empty list is a valid result.
    pub async fn new_news(
        &mut self,
        wildmat: &str,
        since: &NewsDateTime,
    ) -> Result<Vec<String>, Error> {
        let (_, lines) = self
            .reader_data(
                &format!("NEWNEWS {} {}\r\n", token(wildmat)?, since.wire()?),
                230,
            )
            .await?;
        lines
            .iter()
            .map(|line| {
                let id = std::str::from_utf8(line)
                    .map_err(|_| invalid("NEWNEWS returned an invalid Message-ID"))?;
                if !id.starts_with('<') || !id.ends_with('>') || id.len() < 3 {
                    return Err(invalid("NEWNEWS returned an invalid Message-ID"));
                }
                Ok(id.into())
            })
            .collect()
    }

    /// None is bare LIST (equivalent to LIST ACTIVE). Unknown advertised LIST
    /// extensions remain accessible without hard-coding server-specific labels.
    pub async fn list(
        &mut self,
        keyword: Option<&str>,
        argument: Option<&str>,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let mut command = "LIST".to_string();
        if let Some(keyword) = keyword {
            command.push(' ');
            command.push_str(token(keyword)?);
        }
        if let Some(argument) = argument {
            if keyword.is_none() {
                return Err(invalid("LIST argument requires a keyword"));
            }
            command.push(' ');
            command.push_str(token(argument)?);
        }
        command.push_str("\r\n");
        Ok(self.reader_data(&command, 215).await?.1)
    }

    pub async fn list_active_times(
        &mut self,
        wildmat: Option<&str>,
    ) -> Result<Vec<ActiveTime>, Error> {
        self.list(Some("ACTIVE.TIMES"), wildmat)
            .await?
            .iter()
            .map(|line| {
                let text = String::from_utf8_lossy(line);
                let fields = text.split_whitespace().collect::<Vec<_>>();
                if fields.len() != 3 {
                    return Err(invalid("malformed LIST ACTIVE.TIMES entry"));
                }
                Ok(ActiveTime {
                    group: fields[0].into(),
                    created: fields[1]
                        .parse()
                        .map_err(|_| invalid("invalid group creation timestamp"))?,
                    creator: fields[2].into(),
                })
            })
            .collect()
    }

    pub async fn list_newsgroups(
        &mut self,
        wildmat: Option<&str>,
    ) -> Result<Vec<GroupDescription>, Error> {
        self.list(Some("NEWSGROUPS"), wildmat)
            .await?
            .iter()
            .map(|line| {
                let text = String::from_utf8_lossy(line);
                let (name, description) =
                    text.split_once(char::is_whitespace).unwrap_or((&text, ""));
                if name.is_empty() {
                    return Err(invalid("empty newsgroup name"));
                }
                Ok(GroupDescription {
                    name: name.into(),
                    description: description.trim_start().into(),
                })
            })
            .collect()
    }

    pub async fn list_distribution_patterns(&mut self) -> Result<Vec<DistributionPattern>, Error> {
        self.list(Some("DISTRIB.PATS"), None)
            .await?
            .iter()
            .map(|line| {
                let text = String::from_utf8_lossy(line);
                let fields = text.split(':').collect::<Vec<_>>();
                if fields.len() != 3 {
                    return Err(invalid("malformed LIST DISTRIB.PATS entry"));
                }
                Ok(DistributionPattern {
                    weight: fields[0]
                        .parse()
                        .map_err(|_| invalid("invalid distribution weight"))?,
                    pattern: fields[1].into(),
                    distribution: fields[2].into(),
                })
            })
            .collect()
    }
}
