use super::*;
use crate::decode::decode_body_limited;

#[test]
fn independent_python_binascii_fixture_preserves_all_256_octets() {
    let article = include_bytes!("fixtures/all-octets.uue");
    let part = decode_body_limited(article, 256).unwrap();
    assert_eq!(part.data, (0..=255).collect::<Vec<u8>>());
    assert_eq!(part.filename.as_deref(), Some("all octets.bin"));
    assert_eq!(part.file_size, Some(256));
    assert_eq!(part.crc32, 0x29058c73);
}

#[test]
fn historical_known_octets_spaces_and_zero_variants() {
    // Independently known classic UU encodings: Cat -> #0V%T, A -> !00``.
    for zero in [" ", "`"] {
        let encoded = format!("mail preamble\r\nbegin 0644 a file with spaces.jpg\r\n#0V%T\r\n!00``\r\n{zero}\r\nend\r\nmail signature\r\n");
        let part = decode_body_limited(encoded.as_bytes(), 4).unwrap();
        assert_eq!(part.data, b"CatA");
        assert_eq!(part.filename.as_deref(), Some("a file with spaces.jpg"));
        assert_eq!(part.crc32, crc32fast::hash(b"CatA"));
        assert_eq!(part.file_size, Some(4));
    }
}

#[test]
fn known_binary_bytes_and_count_padding() {
    let part = decode_body_limited(b"begin 644 binary.bin\n#`/_]\n`\nend\n", 3).unwrap();
    assert_eq!(part.data, [0, 255, 253]);
}

#[test]
fn rejects_damage_truncation_and_unassembled_multipart() {
    for article in [
        b"begin 888 f\n#0V%T\n`\nend\n".as_slice(),
        b"begin 644 \n#0V%T\n`\nend\n",
        b"begin 644 f\n#0V%\n`\nend\n",
        b"begin 644 f\n#0V%~\n`\nend\n",
        b"begin 644 f\n#0V%Tgarbage\n`\nend\n",
        b"begin 644 f\n#0V%T\nend\n",
        b"begin 644 f\n#0V%T\n`\n",
        b"begin 644 f\n#0V%T\n",
    ] {
        assert!(
            matches!(
                decode_body_limited(article, 100),
                Err(DecodeError::Uuencode(_))
            ),
            "{article:?}"
        );
    }
}

#[test]
fn bounds_are_checked_before_output_growth() {
    let article = b"begin 644 f\n#0V%T\n`\nend\n";
    assert!(matches!(
        decode_body_limited(article, 2),
        Err(DecodeError::TooLarge(2))
    ));
    assert_eq!(decode_body_limited(article, 3).unwrap().data, b"Cat");
    assert!(matches!(
        decode_body_limited(b"begin 644 f\n`\nend\n", 100),
        Err(DecodeError::Empty)
    ));
}

#[test]
fn posix_base64_framing_padding_and_bounds() {
    let article = b"begin-base64 644 a file.bin\r\nQ2F0\r\nQQ==\r\n====\r\n";
    assert_eq!(decode_body_limited(article, 4).unwrap().data, b"CatA");
    assert!(matches!(
        decode_body_limited(article, 3),
        Err(DecodeError::TooLarge(3))
    ));
    for invalid in [
        b"begin-base64 644 f\nQ2F0\n".as_slice(),
        b"begin-base64 644 f\nQ2F\n====\n",
        b"begin-base64 644 f\nQQ==QQ==\n====\n",
        b"begin-base64 644 f\nQ===\n====\n",
    ] {
        assert!(matches!(
            decode_body_limited(invalid, 100),
            Err(DecodeError::Uuencode(_))
        ));
    }
}

#[test]
fn finds_header_beyond_old_512_byte_probe() {
    let mut article = vec![b'x'; 2048];
    article.extend_from_slice(b"\nbegin 644 f\n#0V%T\n`\nend\n");
    assert_eq!(decode_body_limited(&article, 3).unwrap().data, b"Cat");
}

#[test]
fn ordered_multipart_stream_has_one_size_and_no_offset_overwrite() {
    let articles: &[&[u8]] = &[
        b"begin 644 cat and a.bin\r\n#0V%T\r\n",
        b"!00``\r\n",
        b"`\r\nend\r\n",
    ];
    assert!(articles.iter().all(|article| is_fragment(article)));
    let part = decode_parts(articles, 4).unwrap();
    assert_eq!(part.data, b"CatA");
    assert_eq!(part.begin, 0);
    assert_eq!(part.file_size, Some(4));
    assert!(decode_parts(&[articles[0], articles[2]], 100).is_ok());
    // The format carries no checksum/part count: the caller must validate the
    // expected article sequence; decoder validates byte framing and total cap.
    assert!(decode_parts(&[articles[1], articles[2]], 100).is_err());
    assert!(matches!(
        decode_parts(articles, 3),
        Err(DecodeError::TooLarge(3))
    ));
    let complete = b"begin 644 f\n#0V%T\n`\nend\n".as_slice();
    assert!(decode_parts(&[complete, complete], 100).is_err());
}

#[test]
fn streaming_fragments_keep_existing_file_on_failure_and_cleanup_temp() {
    let root = std::env::temp_dir().join(format!(
        "uu-files-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let first = root.join("1");
    let last = root.join("2");
    let output = root.join("file name.bin");
    std::fs::write(&first, b"begin 644 file name.bin\n#0V%T\n").unwrap();
    std::fs::write(&last, b"!00``\n`\nend\n").unwrap();
    let parts = [first.clone(), last.clone()];
    std::fs::write(&output, b"existing").unwrap();
    assert!(write_files(&parts, &output, "wrong.bin", 10).is_err());
    assert!(matches!(
        write_files(&parts, &output, "file name.bin", 3),
        Err(DecodeError::TooLarge(3))
    ));
    assert_eq!(std::fs::read(&output).unwrap(), b"existing");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 3);
    assert_eq!(write_files(&parts, &output, "file name.bin", 4).unwrap(), 4);
    assert_eq!(std::fs::read(&output).unwrap(), b"CatA");
    std::fs::write(&first, b"begin-base64 644 file name.bin\nQ2F0\n").unwrap();
    std::fs::write(&last, b"QQ==\n====\n").unwrap();
    assert_eq!(write_files(&parts, &output, "file name.bin", 4).unwrap(), 4);
    assert_eq!(std::fs::read(&output).unwrap(), b"CatA");
    std::fs::write(&last, b"QQ==\n").unwrap();
    assert!(write_files(&parts, &output, "file name.bin", 4).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"CatA");
    std::fs::remove_dir_all(root).unwrap();
}
