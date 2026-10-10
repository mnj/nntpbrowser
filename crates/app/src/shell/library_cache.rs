//! Filesystem scans run away from egui; unchanged job snapshots reuse the catalog.
use super::*;

#[derive(Clone, PartialEq, Eq)]
struct Source {
    id: i64,
    path: PathBuf,
    title: String,
}

struct Scan {
    generation: u64,
    items: Vec<LibItem>,
}

pub(super) struct LibraryCache {
    pub(super) items: Arc<Vec<LibItem>>,
    pub(super) images: Arc<Vec<usize>>,
    sources: Vec<Source>,
    generation: u64,
    inflight: Option<u64>,
    pending: Option<Vec<Source>>,
    tx: Sender<Scan>,
    rx: Receiver<Scan>,
}

impl Default for LibraryCache {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            items: Arc::new(Vec::new()),
            images: Arc::new(Vec::new()),
            sources: Vec::new(),
            generation: 0,
            inflight: None,
            pending: None,
            tx,
            rx,
        }
    }
}

impl LibraryCache {
    pub(super) fn loading(&self) -> bool {
        self.inflight.is_some()
    }

    pub(super) fn request(&mut self, jobs: &[JobRow], force: bool) {
        let mut sources: Vec<Source> = jobs
            .iter()
            .filter(|job| job.state == "done")
            .map(|job| Source {
                id: job.id,
                path: job.dest_dir.clone().into(),
                title: job.name.clone(),
            })
            .collect();
        sources.sort_by_key(|source| source.id);
        if !force && self.sources == sources {
            return;
        }
        self.sources = sources.clone();
        self.generation = self.generation.wrapping_add(1);
        if self.inflight.is_some() {
            self.pending = Some(sources);
        } else {
            self.spawn(sources);
        }
    }

    fn spawn(&mut self, sources: Vec<Source>) {
        let generation = self.generation;
        self.inflight = Some(generation);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Scan {
                generation,
                items: scan(&sources),
            });
        });
    }

    pub(super) fn poll(&mut self) -> bool {
        let mut updated = false;
        while let Ok(result) = self.rx.try_recv() {
            if self.inflight != Some(result.generation) {
                continue;
            }
            self.inflight = None;
            if result.generation == self.generation {
                self.images = Arc::new(
                    result
                        .items
                        .iter()
                        .enumerate()
                        .filter(|(_, item)| item.kind == LibKind::Image)
                        .map(|(index, _)| index)
                        .collect(),
                );
                self.items = Arc::new(result.items);
                updated = true;
            }
            if let Some(sources) = self.pending.take() {
                self.spawn(sources);
            }
        }
        updated
    }
}

fn scan(sources: &[Source]) -> Vec<LibItem> {
    let mut items = std::collections::BTreeMap::new();
    for source in sources {
        let mut directories = vec![source.path.clone()];
        while let Some(directory) = directories.pop() {
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    if entry.path() == source.path.join(format!(".uu-parts-{}", source.id)) {
                        // A completed job can retain retry staging if its best-effort
                        // cleanup failed. Encoded fragments are not saved output.
                        continue;
                    }
                    directories.push(entry.path());
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                let path = entry.path();
                let ext = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let kind = match ext.as_str() {
                    "jpg" | "jpeg" | "png" | "gif" | "webp" => LibKind::Image,
                    "mp4" | "mkv" | "webm" | "mov" | "avi" => LibKind::Video,
                    "rar" | "zip" | "7z" => LibKind::Packed,
                    _ => LibKind::File,
                };
                items.insert(
                    path.clone(),
                    LibItem {
                        job_id: source.id,
                        path,
                        title: source.title.clone(),
                        kind,
                        size: entry.metadata().map(|metadata| metadata.len()).unwrap_or(0),
                    },
                );
            }
        }
    }
    items.into_values().collect()
}
