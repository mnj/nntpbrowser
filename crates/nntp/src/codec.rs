use crate::error::Error;
use crate::types::{Capabilities, GroupInfo, GroupStatus, OverviewLine};

/// Column indexes into overview fields after the article number.
#[derive(Debug, Clone)]
pub(crate) struct OverviewFmt {
    pub subject: usize,
    pub from: usize,
    pub date: usize,
    pub message_id: usize,
    pub references: usize,
    pub bytes: usize,
    pub lines: usize,
}

impl Default for OverviewFmt {
    fn default() -> Self {
        Self::positional()
    }
}

impl OverviewFmt {
    pub(crate) fn positional() -> Self {
        Self {
            subject: 0,
            from: 1,
            date: 2,
            message_id: 3,
            references: 4,
            bytes: 5,
            lines: 6,
        }
    }

    /// Names come from `LIST OVERVIEW.FMT`. A missing required name selects the
    /// historical 0..6 layout (article number is not part of the fmt list).
    pub(crate) fn from_header_names(names: &[String]) -> Self {
        let norm: Vec<String> = names.iter().map(|n| normalize_overview_name(n)).collect();
        let find = |key: &str| norm.iter().position(|n| n == key);
        match (
            find("subject"),
            find("from"),
            find("date"),
            find("message-id"),
            find("references"),
            find("bytes"),
            find("lines"),
        ) {
            (
                Some(subject),
                Some(from),
                Some(date),
                Some(message_id),
                Some(references),
                Some(bytes),
                Some(lines),
            ) => Self {
                subject,
                from,
                date,
                message_id,
                references,
                bytes,
                lines,
            },
            _ => Self::positional(),
        }
    }
}

pub(crate) fn normalize_overview_name(name: &str) -> String {
    name.trim()
        .trim_start_matches(':')
        .trim_end_matches(':')
        .trim()
        .to_ascii_lowercase()
}

pub(crate) fn parse_overview_line(line: &[u8], fmt: &OverviewFmt) -> Result<OverviewLine, Error> {
    let text = String::from_utf8_lossy(line);
    let fields: Vec<&str> = text.split('\t').collect();
    if fields.is_empty() || fields[0].trim().is_empty() {
        return Err(Error::Protocol {
            code: 0,
            text: "empty overview line".into(),
        });
    }
    let article: u64 = fields[0].trim().parse().map_err(|_| Error::Protocol {
        code: 0,
        text: format!("bad article number '{}'", fields[0]),
    })?;
    let take = |idx: usize| -> String { fields.get(idx + 1).copied().unwrap_or("").to_string() };
    let parse_u64 = |idx: usize| -> u64 { take(idx).trim().parse().unwrap_or(0) };
    Ok(OverviewLine {
        article,
        subject: take(fmt.subject),
        from: take(fmt.from),
        date: take(fmt.date),
        message_id: take(fmt.message_id),
        references: take(fmt.references),
        bytes: parse_u64(fmt.bytes),
        lines: parse_u64(fmt.lines),
    })
}

pub(crate) fn parse_capabilities(lines: &[Vec<u8>]) -> Capabilities {
    let mut caps = Capabilities::empty();
    for line in lines {
        let text = String::from_utf8_lossy(line);
        let mut parts = text.split_whitespace();
        let Some(name) = parts.next() else {
            continue;
        };
        let args: Vec<String> = parts.map(|s| s.to_string()).collect();
        let name_upper = name.to_ascii_uppercase();
        if name_upper == "COMPRESS" && args.iter().any(|a| a.eq_ignore_ascii_case("DEFLATE")) {
            caps.compress_deflate = true;
        }
        if name_upper == "OVER" || name_upper == "XOVER" {
            caps.over = true;
        }
        if name_upper == "MODE-READER" {
            caps.mode_reader = true;
        }
        caps.entries.push((name.to_string(), args));
    }
    caps
}

pub(crate) fn parse_status_line(line: &[u8]) -> Result<(u16, String), Error> {
    let text = std::str::from_utf8(line).map_err(|_| Error::Protocol {
        code: 0,
        text: "status line is not utf-8".into(),
    })?;
    let text = text.trim_end_matches(['\r', '\n']);
    if text.len() < 3
        || !text.as_bytes()[..3].iter().all(u8::is_ascii_digit)
        || (text.len() > 3 && text.as_bytes()[3] != b' ')
    {
        return Err(Error::Protocol {
            code: 0,
            text: format!("malformed status: {text}"),
        });
    }
    let code: u16 = text[..3].parse().map_err(|_| Error::Protocol {
        code: 0,
        text: format!("malformed status: {text}"),
    })?;
    let rest = if text.len() >= 4 {
        text[4..].to_string()
    } else {
        String::new()
    };
    Ok((code, rest))
}

/// RFC 3977 section 3.1 limits the entire command to 512 octets including
/// CRLF, and its arguments to 497 octets. SASL uses separate extension limits.
pub(crate) fn validate_command(data: &[u8]) -> Result<(), Error> {
    if data.len() > 512 {
        // Command arguments can contain credentials; do not include them here.
        return Err(Error::Protocol {
            code: 0,
            text: "NNTP command exceeds 512 octets including CRLF".into(),
        });
    }
    let line = data.strip_suffix(b"\r\n").unwrap_or(data);
    if let Some(separator) = line.iter().position(|byte| matches!(byte, b' ' | b'\t')) {
        let arguments = &line[separator + 1..];
        let leading_space = arguments
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        if arguments.len() - leading_space > 497 {
            return Err(Error::Protocol {
                code: 0,
                text: "NNTP command arguments exceed 497 octets".into(),
            });
        }
    }
    Ok(())
}

pub(crate) fn parse_group_status(text: &str, fallback_name: &str) -> Result<GroupStatus, Error> {
    let mut parts = text.split_whitespace();
    let count = parse_u64_token(parts.next(), "count")?;
    let low = parse_u64_token(parts.next(), "low")?;
    let high = parse_u64_token(parts.next(), "high")?;
    let name = parts.next().unwrap_or(fallback_name).to_string();
    Ok(GroupStatus {
        count,
        low,
        high,
        name,
    })
}

/// `LIST ACTIVE` lines are `name high low status` (high before low).
pub(crate) fn parse_group_info(line: &[u8]) -> Result<GroupInfo, Error> {
    let text = String::from_utf8_lossy(line);
    let mut parts = text.split_whitespace();
    let name = parts.next().ok_or_else(|| Error::Protocol {
        code: 0,
        text: "short LIST ACTIVE line".into(),
    })?;
    let high = parse_u64_token(parts.next(), "high")?;
    let low = parse_u64_token(parts.next(), "low")?;
    let status = parts
        .next()
        .and_then(|s| s.chars().next())
        .ok_or_else(|| Error::Protocol {
            code: 0,
            text: "LIST ACTIVE line missing status".into(),
        })?;
    Ok(GroupInfo {
        name: name.to_string(),
        high,
        low,
        status,
    })
}

fn parse_u64_token(token: Option<&str>, what: &str) -> Result<u64, Error> {
    let Some(token) = token else {
        return Err(Error::Protocol {
            code: 0,
            text: format!("missing {what}"),
        });
    };
    token.parse().map_err(|_| Error::Protocol {
        code: 0,
        text: format!("bad {what}: {token}"),
    })
}

/// `None` is the terminating dot. A leading dot on any other line is unstuffed.
pub(crate) fn unstuff(line: &[u8]) -> Option<&[u8]> {
    if line == b"." {
        None
    } else if line.first() == Some(&b'.') {
        Some(&line[1..])
    } else {
        Some(line)
    }
}

pub(crate) fn pop_line(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let pos = buf.iter().position(|b| *b == b'\n')?;
    let mut line: Vec<u8> = buf.drain(..=pos).collect();
    line.pop();
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Some(line)
}

/// Dot-unstuffed body lines joined with CRLF. Empty input stays empty.
pub(crate) fn join_lines(lines: &[Vec<u8>]) -> Vec<u8> {
    if lines.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(line);
    }
    out.extend_from_slice(b"\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parser_checks_separator_before_slicing_unicode() {
        for line in [
            "200é".as_bytes(),
            "200🙂".as_bytes(),
            b"200X",
            b"200\tready",
            b"20",
            b"2x0 ready",
            b"200 \xff",
        ] {
            assert!(matches!(
                parse_status_line(line),
                Err(Error::Protocol { .. })
            ));
        }
        assert_eq!(parse_status_line(b"200").unwrap(), (200, String::new()));
        assert_eq!(
            parse_status_line(b"200 \r\n").unwrap(),
            (200, String::new())
        );
        assert_eq!(
            parse_status_line("200 résumé ready\r\n".as_bytes()).unwrap(),
            (200, "résumé ready".into())
        );
    }

    #[test]
    fn command_limit_counts_octets_and_terminating_crlf() {
        let mut command = b"XTESTCOMMAND ".to_vec();
        command.extend_from_slice("é".repeat(248).as_bytes());
        command.push(b'a');
        command.extend_from_slice(b"\r\n");
        assert_eq!(command.len(), 512);
        assert!(validate_command(&command).is_ok());
        command.insert(13, b'a');
        assert_eq!(command.len(), 513);
        assert!(matches!(
            validate_command(&command),
            Err(Error::Protocol { .. })
        ));
    }

    #[test]
    fn argument_limit_is_checked_independently_of_short_command_length() {
        for separator in [" ", "\t", "  \t"] {
            let accepted = format!("BODY{separator}{}\r\n", "é".repeat(248) + "a");
            assert!(validate_command(accepted.as_bytes()).is_ok());
            let rejected = format!("BODY{separator}{}\r\n", "é".repeat(249));
            assert!(rejected.len() <= 512);
            let error = validate_command(rejected.as_bytes()).unwrap_err();
            assert!(error.to_string().contains("497"));
            assert!(!error.to_string().contains('é'));
        }
    }

    #[test]
    fn oversized_command_error_does_not_echo_credentials() {
        let command = format!("AUTHINFO PASS {}\r\n", "secret-password".repeat(40));
        let error = validate_command(command.as_bytes()).unwrap_err();
        assert!(!error.to_string().contains("secret-password"));
    }

    #[test]
    fn overview_fmt_strips_colons_and_falls_back() {
        let names = vec![
            "Subject:".into(),
            "From:".into(),
            "Date:".into(),
            "Message-ID:".into(),
            "References:".into(),
            ":bytes".into(),
            ":lines".into(),
            "Xref:full".into(),
        ];
        let fmt = OverviewFmt::from_header_names(&names);
        assert_eq!(fmt.subject, 0);
        assert_eq!(fmt.bytes, 5);
        assert_eq!(fmt.lines, 6);

        let broken = vec!["Subject:".into(), "From:".into()];
        let fmt = OverviewFmt::from_header_names(&broken);
        assert_eq!(fmt.message_id, 3);
    }

    #[test]
    fn unstuff_dot_line() {
        assert_eq!(unstuff(b"."), None);
        assert_eq!(unstuff(b"..hidden"), Some(b".hidden".as_slice()));
        assert_eq!(unstuff(b".1\t.sub"), Some(b"1\t.sub".as_slice()));
        assert_eq!(unstuff(b"1\tok"), Some(b"1\tok".as_slice()));
    }
}
