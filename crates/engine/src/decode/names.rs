// Subjects and attachment headers can differ in ASCII case, outer quoting or a
// posting client's source directory. Compare complete leaf names only; the
// subject-derived identity remains the sole filename used by explicit Save.
pub(crate) fn same_attachment_name(body: &str, header: &str) -> bool {
    fn leaf(name: &str) -> Option<&str> {
        let name = name.trim();
        let name = if (name.starts_with('"') && name.ends_with('"'))
            || (name.starts_with('\'') && name.ends_with('\''))
        {
            name.get(1..name.len().checked_sub(1)?)?
        } else {
            name
        };
        let name = name.rsplit(['/', '\\']).next()?.trim();
        (!name.is_empty() && !matches!(name, "." | "..") && !name.chars().any(char::is_control))
            .then_some(name)
    }
    match (leaf(body), leaf(header)) {
        (Some(body), Some(header)) => body.eq_ignore_ascii_case(header),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_attachment_names_allow_poster_formatting_but_not_other_files() {
        for name in [
            "My holiday.JPG",
            "  My holiday.jpg  ",
            "\"My holiday.jpg\"",
            "C:\\Photos\\My holiday.jpg",
            "/photos/My holiday.jpg",
        ] {
            assert!(same_attachment_name(name, "My holiday.jpg"), "{name:?}");
        }
        for name in [
            "holiday.jpg",
            "Other holiday.jpg",
            "My holiday.png",
            "",
            "..",
            "../",
        ] {
            assert!(!same_attachment_name(name, "My holiday.jpg"), "{name:?}");
        }
        assert!(!same_attachment_name("My\0 holiday.jpg", "My holiday.jpg"));
    }
}
