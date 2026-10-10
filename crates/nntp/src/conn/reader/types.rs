//! Typed RFC 3977 reader command arguments and responses.

use crate::{ArticleRef, Error, GroupStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleIdentity {
    /// May be zero for retrieval by Message-ID without a selected group.
    pub number: u64,
    /// Canonical wire form, including angle brackets.
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Article {
    pub identity: ArticleIdentity,
    /// Dot-unstuffed article, including headers, separator and body, with CRLF.
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArticleRange {
    pub first: u64,
    /// None requests all numbers from first onwards; equal to first requests one.
    pub last: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeRef {
    Current,
    Article(ArticleRef),
    Range(ArticleRange),
}

#[derive(Debug, Clone)]
pub struct ListGroup {
    pub group: GroupStatus,
    pub articles: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderValue {
    /// Zero is legal when HDR is requested by Message-ID.
    pub article: u64,
    pub value: String,
}

/// A header or metadata value retaining legacy non-UTF-8 octets exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderBytes {
    /// Zero is legal when HDR is requested by Message-ID.
    pub article: u64,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderScope {
    All,
    MessageId,
    Range,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTime {
    pub group: String,
    pub created: u64,
    pub creator: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDescription {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistributionPattern {
    pub weight: u64,
    pub pattern: String,
    pub distribution: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsDateTime {
    /// Six or eight digits; prefer the unambiguous four-digit year form.
    pub date: String,
    pub time: String,
    /// false uses the server's local timezone; true appends GMT (UTC).
    pub gmt: bool,
}

pub(super) fn token(value: &str) -> Result<&str, Error> {
    if value.is_empty() || value.bytes().any(|b| b <= b' ' || b == 127) {
        Err(invalid("command argument must be a nonempty token"))
    } else {
        Ok(value)
    }
}

pub(super) fn invalid(text: &str) -> Error {
    Error::Protocol {
        code: 0,
        text: text.into(),
    }
}

impl ArticleRange {
    pub(super) fn wire(self) -> Result<String, Error> {
        if self.first == 0 || self.last.is_some_and(|last| last < self.first) {
            return Err(invalid("invalid article range"));
        }
        Ok(match self.last {
            Some(last) if last == self.first => self.first.to_string(),
            Some(last) => format!("{}-{last}", self.first),
            None => format!("{}-", self.first),
        })
    }
}

impl NewsDateTime {
    pub(super) fn wire(&self) -> Result<String, Error> {
        if !matches!(self.date.len(), 6 | 8)
            || self.time.len() != 6
            || !self.date.bytes().all(|b| b.is_ascii_digit())
            || !self.time.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid("date/time must be yyyymmdd (or yymmdd) and hhmmss"));
        }
        let n = |s: &str| s.parse::<u32>().unwrap_or(0);
        let month = n(&self.date[self.date.len() - 4..self.date.len() - 2]);
        let day = n(&self.date[self.date.len() - 2..]);
        let year = n(&self.date[..self.date.len() - 4]);
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days = match month {
            2 if leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => 0,
        };
        if day == 0
            || day > days
            || n(&self.time[..2]) > 23
            || n(&self.time[2..4]) > 59
            || n(&self.time[4..]) > 60
            || (self.date.len() == 8 && year < 1900)
        {
            return Err(invalid("invalid date/time"));
        }
        Ok(format!(
            "{} {}{}",
            self.date,
            self.time,
            if self.gmt { " GMT" } else { "" }
        ))
    }
}
