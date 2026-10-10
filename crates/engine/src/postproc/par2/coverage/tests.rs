use super::*;
use std::io::Cursor;

const SET: [u8; 16] = [0x11; 16];

fn packet(set: [u8; 16], exponent: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"PAR2\0PKT");
    bytes.extend_from_slice(&72u64.to_le_bytes());
    bytes.extend_from_slice(&[0; 16]);
    bytes.extend_from_slice(&set);
    bytes.extend_from_slice(b"PAR 2.0\0RecvSlic");
    bytes.extend_from_slice(&exponent.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes
}

fn covered(bytes: Vec<u8>, range: (u64, u64)) -> bool {
    let len = bytes.len() as u64;
    covers(&mut Cursor::new(bytes), len, &SET, 4, range, true).unwrap()
}

#[test]
fn full_range_requires_distinct_actual_exponents_not_hint_or_packet_count() {
    assert!(covered([packet(SET, 0), packet(SET, 1)].concat(), (0, 2)));
    assert!(!covered(packet(SET, 0), (0, 100)));
    assert!(!covered([packet(SET, 0), packet(SET, 0)].concat(), (0, 2)));
    assert!(!covered(
        [packet(SET, 0), packet([0x22; 16], 1)].concat(),
        (0, 2)
    ));
    assert!(!covered([packet(SET, 0), packet(SET, 1)].concat(), (1, 3)));
    assert!(!covered(Vec::new(), (0, MAX_EXPONENTS + 1)));
    assert!(!covered(Vec::new(), (10, 9)));
}

#[test]
fn malformed_truncated_or_oversized_packets_do_not_establish_coverage() {
    let normal = packet(SET, 0);
    assert!(!covered(normal[..normal.len() - 1].to_vec(), (0, 1)));
    assert!(!covered([normal.clone(), vec![0; 3]].concat(), (0, 1)));
    let mut bad = normal.clone();
    bad[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(!covered(bad, (0, 1)));
    let mut bad = normal.clone();
    bad[8..16].copy_from_slice(&68u64.to_le_bytes());
    assert!(!covered(bad, (0, 1)));
    let mut bad = normal.clone();
    bad[0] = 0;
    assert!(!covered(bad, (0, 1)));
    let mut bad = normal;
    bad[8..16].copy_from_slice(&73u64.to_le_bytes());
    assert!(!covered(bad, (0, 1)));
}

#[test]
fn scanner_skips_large_recovery_payloads_instead_of_buffering_them() {
    struct HeaderOnly {
        bytes: Cursor<Vec<u8>>,
        reads: usize,
    }
    impl Read for HeaderOnly {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            assert!(
                self.bytes.position() < 68,
                "scanner attempted to read payload bytes"
            );
            self.reads += buf.len();
            self.bytes.read(buf)
        }
    }
    impl Seek for HeaderOnly {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.bytes.seek(pos)
        }
    }
    let slice = 4 * 1024 * 1024u64;
    let mut header = packet(SET, 7);
    header.truncate(68);
    header[8..16].copy_from_slice(&(68 + slice).to_le_bytes());
    let mut reader = HeaderOnly {
        bytes: Cursor::new(header),
        reads: 0,
    };
    assert!(covers(&mut reader, 68 + slice, &SET, slice, (7, 8), true).unwrap());
    assert_eq!(reader.reads, 68);
}

#[test]
fn file_helper_rejects_links_and_wrong_set_identity() {
    let path = std::env::temp_dir().join(format!(
        "nntpbrowser-par-coverage-{}-{}.par2",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, packet(SET, 0)).unwrap();
    assert!(volume_covers_range(&path, &"11".repeat(16), 4, (0, 1)));
    assert!(!volume_covers_range(&path, &"22".repeat(16), 4, (0, 1)));
    assert!(!volume_covers_range(&path, "not-a-set", 4, (0, 1)));
    #[cfg(unix)]
    {
        let link = path.with_extension("link.par2");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(!volume_covers_range(&link, &"11".repeat(16), 4, (0, 1)));
        fs::remove_file(link).unwrap();
    }
    fs::remove_file(path).unwrap();
}

#[test]
fn golden_recovery_volume_confirms_only_its_actual_range() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/runtime/tests/fixtures/par2/payload.vol0+1.par2");
    let set = rust_par2::parse(&path).unwrap();
    let id: String = set
        .recovery_set_id
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert!(volume_covers_range(&path, &id, set.slice_size, (0, 1)));
    assert!(!volume_covers_range(&path, &id, set.slice_size, (0, 100)));
}

#[test]
fn overlap_requires_actual_exponents_and_validates_trailing_packets() {
    let path = std::env::temp_dir().join(format!(
        "nntpbrowser-misnamed-{}.vol0+1.par2",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // The filename advertises zero, but this volume contains only exponent 10.
    fs::write(&path, packet(SET, 10)).unwrap();
    let id = "11".repeat(16);
    assert!(!volume_overlaps_range(&path, &id, 4, (0, 1)));
    assert!(volume_overlaps_range(&path, &id, 4, (10, 12)));
    assert!(!volume_covers_range(&path, &id, 4, (10, 12)));
    assert!(!volume_overlaps_range(&path, &"22".repeat(16), 4, (10, 12)));
    fs::write(&path, [packet(SET, 10), vec![0; 3]].concat()).unwrap();
    assert!(!volume_overlaps_range(&path, &id, 4, (10, 12)));
    fs::remove_file(path).unwrap();
}
