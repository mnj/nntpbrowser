use super::*;

const MAGIC: &[u8; 4] = b"OSF1";
const VERSION: u16 = 1;
pub(super) const HEADER_LEN: usize = 34;
pub(super) const DEFAULT_BLOCK: usize = 256 * 1024;

#[derive(Clone, Copy)]
struct IndexEntry {
    first: u64,
    offset: u64,
}

/// Writes OSF1 incrementally without retaining all overview records.
pub(super) struct OsfWriter {
    file: File,
    low: u64,
    high: u64,
    block_target: usize,
    raw: Vec<u8>,
    block_first: u64,
    previous: Option<u64>,
    index: Vec<u8>,
    blocks: u32,
}

impl OsfWriter {
    pub(super) fn new(
        path: &Path,
        low: u64,
        high: u64,
        block_target: usize,
    ) -> Result<Self, Error> {
        let mut file = create_file(path)?;
        file.write_all(&header_bytes(low, high, 0, 0))?;
        Ok(Self {
            file,
            low,
            high,
            block_target,
            raw: Vec::with_capacity(block_target),
            block_first: 0,
            previous: None,
            index: Vec::new(),
            blocks: 0,
        })
    }

    pub(super) fn push(&mut self, record: &Record) -> Result<(), Error> {
        if self
            .previous
            .is_some_and(|previous| record.article <= previous)
        {
            return Err(Error::Order);
        }
        let size = encoded_record_len(record);
        if size > 4 * 1024 * 1024 {
            return Err(Error::Header("overview record exceeds block limit".into()));
        }
        if !self.raw.is_empty() && self.raw.len() + size > self.block_target {
            self.flush_block()?;
        }
        if self.raw.is_empty() {
            self.block_first = record.article;
        }
        encode_record_into(&mut self.raw, record);
        self.previous = Some(record.article);
        Ok(())
    }

    fn flush_block(&mut self) -> Result<(), Error> {
        if self.raw.is_empty() {
            return Ok(());
        }
        let compressed =
            zstd::bulk::compress(&self.raw, 3).map_err(|err| Error::Header(err.to_string()))?;
        let offset = self.file.stream_position()?;
        self.index
            .extend_from_slice(&self.block_first.to_le_bytes());
        self.index.extend_from_slice(&offset.to_le_bytes());
        self.file
            .write_all(&(self.raw.len() as u32).to_le_bytes())?;
        self.file
            .write_all(&(compressed.len() as u32).to_le_bytes())?;
        self.file.write_all(&xxh3_64(&self.raw).to_le_bytes())?;
        self.file.write_all(&compressed)?;
        self.raw.clear();
        self.blocks = self
            .blocks
            .checked_add(1)
            .ok_or_else(|| Error::Header("too many header blocks".into()))?;
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<(), Error> {
        self.flush_block()?;
        let index_offset = self.file.stream_position()?;
        self.file.write_all(&self.index)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&header_bytes(
            self.low,
            self.high,
            self.blocks,
            index_offset,
        ))?;
        // Persist the range before publishing it in the manifest.
        self.file.sync_all()?;
        Ok(())
    }
}

pub(super) fn write_osf(
    path: &Path,
    low: u64,
    high: u64,
    records: &[Record],
    block_target: usize,
) -> Result<(), Error> {
    let mut writer = OsfWriter::new(path, low, high, block_target)?;
    for record in records {
        writer.push(record)?;
    }
    writer.finish()
}

fn header_bytes(low: u64, high: u64, blocks: u32, index_offset: u64) -> [u8; HEADER_LEN] {
    let mut out = [0u8; HEADER_LEN];
    out[..4].copy_from_slice(MAGIC);
    out[4..6].copy_from_slice(&VERSION.to_le_bytes());
    out[6..14].copy_from_slice(&low.to_le_bytes());
    out[14..22].copy_from_slice(&high.to_le_bytes());
    out[22..26].copy_from_slice(&blocks.to_le_bytes());
    out[26..34].copy_from_slice(&index_offset.to_le_bytes());
    out
}

pub(super) fn read_osf_range<F>(
    path: &Path,
    low: u64,
    high: u64,
    on_record: &mut F,
) -> Result<Vec<BadSpan>, Error>
where
    F: FnMut(Record),
{
    let mut file = File::open(path)?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)?;
    if &header[..4] != MAGIC {
        return Err(Error::Header(format!(
            "{} is not an OSF1 file",
            path.display()
        )));
    }
    let version = u16::from_le_bytes(header[4..6].try_into().unwrap());
    if version != VERSION {
        return Err(Error::Header(format!("unsupported OSF version {version}")));
    }
    let file_low = u64::from_le_bytes(header[6..14].try_into().unwrap());
    let file_high = u64::from_le_bytes(header[14..22].try_into().unwrap());
    let block_count = u32::from_le_bytes(header[22..26].try_into().unwrap()) as usize;
    let index_offset = u64::from_le_bytes(header[26..34].try_into().unwrap());
    let file_len = file.metadata()?.len();
    let index_len = (block_count as u64)
        .checked_mul(16)
        .ok_or_else(|| Error::Header("invalid OSF index length".into()))?;
    if block_count == 0
        || file_low > file_high
        || index_offset < HEADER_LEN as u64
        || index_offset.checked_add(index_len) != Some(file_len)
    {
        return Err(Error::Header("invalid OSF index bounds".into()));
    }
    file.seek(SeekFrom::Start(index_offset))?;
    let mut index_buf = vec![0u8; index_len as usize];
    file.read_exact(&mut index_buf)?;
    let mut index = Vec::with_capacity(block_count);
    for chunk in index_buf.as_chunks::<16>().0 {
        index.push(IndexEntry {
            first: u64::from_le_bytes(chunk[..8].try_into().unwrap()),
            offset: u64::from_le_bytes(chunk[8..].try_into().unwrap()),
        });
    }
    if index.iter().any(|entry| {
        entry.first < file_low
            || entry.first > file_high
            || entry.offset < HEADER_LEN as u64
            || entry.offset >= index_offset
    }) || index
        .windows(2)
        .any(|pair| pair[0].first >= pair[1].first || pair[0].offset >= pair[1].offset)
    {
        return Err(Error::Header("invalid OSF block index".into()));
    }
    let mut bad = Vec::new();
    // Binary search the preceding block; a narrow lookup need not walk the
    // entire retained group's index.
    let start = index
        .partition_point(|entry| entry.first <= low)
        .saturating_sub(1);
    for (i, entry) in index.iter().enumerate().skip(start) {
        if entry.first > high {
            break;
        }
        let last = index
            .get(i + 1)
            .map(|next| next.first.saturating_sub(1))
            .unwrap_or(file_high);
        if last < low || entry.first > high {
            continue;
        }
        let block_end = index.get(i + 1).map_or(index_offset, |next| next.offset);
        match decode_block(&mut file, entry.offset, block_end, entry.first, last) {
            Ok(records) => {
                for record in records {
                    if record.article >= low && record.article <= high {
                        on_record(record);
                    }
                }
            }
            Err(_) => bad.push(BadSpan {
                first: entry.first,
                last,
            }),
        }
    }
    Ok(bad)
}

fn decode_block(
    file: &mut File,
    offset: u64,
    block_end: u64,
    first: u64,
    last: u64,
) -> Result<Vec<Record>, Error> {
    file.seek(SeekFrom::Start(offset))?;
    let mut meta = [0u8; 16];
    file.read_exact(&mut meta)?;
    let uncomp = u32::from_le_bytes(meta[..4].try_into().unwrap()) as usize;
    let comp_len = u32::from_le_bytes(meta[4..8].try_into().unwrap()) as usize;
    let hash = u64::from_le_bytes(meta[8..16].try_into().unwrap());
    if uncomp > 4 * 1024 * 1024
        || comp_len > 8 * 1024 * 1024
        || offset.checked_add(16 + comp_len as u64) != Some(block_end)
    {
        return Err(Error::Header(format!("block {first}-{last} is oversized")));
    }
    let mut compressed = vec![0u8; comp_len];
    file.read_exact(&mut compressed)?;
    let raw = zstd::bulk::decompress(&compressed, uncomp)
        .map_err(|err| Error::Header(err.to_string()))?;
    if raw.len() != uncomp || xxh3_64(&raw) != hash {
        return Err(Error::Header(format!("block {first}-{last} failed xxh3")));
    }
    let records = decode_records(&raw)?;
    if records.first().is_none_or(|record| record.article != first)
        || records.last().is_some_and(|record| record.article > last)
    {
        return Err(Error::Header("block article bounds".into()));
    }
    Ok(records)
}

fn encoded_record_len(record: &Record) -> usize {
    29 + 12
        + record.message_id.len()
        + record.subject.len()
        + record.from.len()
        + record.date_raw.as_ref().map_or(0, |raw| 4 + raw.len())
}

fn encode_record_into(out: &mut Vec<u8>, record: &Record) {
    out.extend_from_slice(&record.article.to_le_bytes());
    out.extend_from_slice(&record.date_unix.to_le_bytes());
    out.extend_from_slice(&record.bytes.to_le_bytes());
    out.extend_from_slice(&record.lines.to_le_bytes());
    let mut flags = 0u8;
    if record.date_raw.is_some() {
        flags |= 1;
    }
    out.push(flags);
    write_str(out, &record.message_id);
    write_str(out, &record.subject);
    write_str(out, &record.from);
    if let Some(raw) = &record.date_raw {
        write_str(out, raw);
    }
}

fn decode_records(mut raw: &[u8]) -> Result<Vec<Record>, Error> {
    let mut out = Vec::new();
    let mut prev = 0u64;
    while !raw.is_empty() {
        if raw.len() < 8 + 8 + 8 + 4 + 1 {
            return Err(Error::Header("truncated record".into()));
        }
        let article = u64::from_le_bytes(raw[..8].try_into().unwrap());
        raw = &raw[8..];
        let date_unix = i64::from_le_bytes(raw[..8].try_into().unwrap());
        raw = &raw[8..];
        let bytes = u64::from_le_bytes(raw[..8].try_into().unwrap());
        raw = &raw[8..];
        let lines = u32::from_le_bytes(raw[..4].try_into().unwrap());
        raw = &raw[4..];
        let flags = raw[0];
        raw = &raw[1..];
        let message_id = read_str(&mut raw)?;
        let subject = read_str(&mut raw)?;
        let from = read_str(&mut raw)?;
        let date_raw = if flags & 1 == 1 {
            Some(read_str(&mut raw)?)
        } else {
            None
        };
        if !out.is_empty() && article <= prev {
            return Err(Error::Header("record order".into()));
        }
        prev = article;
        out.push(Record {
            article,
            date_unix,
            date_raw,
            bytes,
            lines,
            message_id,
            subject,
            from,
        });
    }
    Ok(out)
}

fn write_str(out: &mut Vec<u8>, text: &str) {
    let bytes = text.as_bytes();
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn read_str(raw: &mut &[u8]) -> Result<String, Error> {
    if raw.len() < 4 {
        return Err(Error::Header("truncated string".into()));
    }
    let len = u32::from_le_bytes(raw[..4].try_into().unwrap()) as usize;
    *raw = &raw[4..];
    if raw.len() < len {
        return Err(Error::Header("truncated string bytes".into()));
    }
    let text = std::str::from_utf8(&raw[..len])
        .map_err(|err| Error::Header(err.to_string()))?
        .to_string();
    *raw = &raw[len..];
    Ok(text)
}
