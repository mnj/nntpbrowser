//! Own-process telemetry: one background sampler and one latest snapshot.
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Clone, Copy)]
pub(super) struct Snapshot {
    cpu: Option<f32>,
    resident: u64,
}

impl Snapshot {
    fn values(self) -> (String, String) {
        let cpu = self
            .cpu
            .filter(|cpu| cpu.is_finite() && *cpu >= 0.0)
            .map_or_else(|| "—".into(), |cpu| format!("{cpu:.1}%"));
        let memory = resident_label(self.resident);
        (cpu, memory)
    }
}

fn resident_label(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit < 3 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[derive(Default)]
pub(super) struct ProcessStats {
    latest: Arc<Mutex<Option<Snapshot>>>,
    // Dropping this sender wakes the worker immediately, even between samples.
    _stop: Option<mpsc::Sender<()>>,
}

impl ProcessStats {
    pub(super) fn start() -> Self {
        if !sysinfo::IS_SUPPORTED_SYSTEM {
            return Self::default();
        }
        let pid = Pid::from_u32(std::process::id());
        let mut system = None;
        let mut primed = false;
        let sample = move || {
            // System setup and all OS reads stay inside the sampling thread.
            let system = system.get_or_insert_with(System::new);
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing().with_cpu().with_memory(),
            );
            let snapshot = system.process(pid).map(|process| Snapshot {
                cpu: primed.then(|| process.cpu_usage()),
                resident: process.memory(),
            });
            primed = true;
            snapshot
        };
        Self::spawn(sample, Duration::from_secs(1))
            .map_or_else(|_| Self::default(), |(stats, _)| stats)
    }

    fn spawn(
        mut sample: impl FnMut() -> Option<Snapshot> + Send + 'static,
        interval: Duration,
    ) -> std::io::Result<(Self, std::thread::JoinHandle<()>)> {
        let stats = Self::default();
        let latest = Arc::downgrade(&stats.latest);
        let (stop, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("process-stats".into())
            .spawn(move || loop {
                let snapshot = sample();
                let Some(latest) = latest.upgrade() else {
                    break;
                };
                if let Ok(mut value) = latest.lock() {
                    *value = snapshot;
                }
                drop(latest);
                match receiver.recv_timeout(interval) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
            })?;
        Ok((
            Self {
                _stop: Some(stop),
                ..stats
            },
            worker,
        ))
    }

    pub(super) fn values(&self) -> (String, String) {
        self.latest
            .try_lock()
            .ok()
            .and_then(|value| *value)
            .map_or_else(|| ("—".into(), "—".into()), Snapshot::values)
    }

    #[cfg(feature = "ui-preview")]
    pub(super) fn fixture() -> Self {
        Self {
            latest: Arc::new(Mutex::new(Some(Snapshot {
                cpu: Some(8.4),
                resident: 286 * 1024 * 1024,
            }))),
            _stop: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_cpu_keeps_multicore_percent_and_resident_bytes_keep_their_units() {
        let snapshot = Snapshot {
            cpu: Some(245.5),
            resident: 512 * 1024 * 1024,
        };
        assert_eq!(snapshot.values(), ("245.5%".into(), "512 MiB".into()));
        for invalid in [f32::NAN, f32::INFINITY, -1.0] {
            assert!(
                Snapshot {
                    cpu: Some(invalid),
                    resident: 1024
                }
                .values()
                .0 == "—"
            );
        }
        assert_eq!(
            Snapshot {
                cpu: None,
                resident: 1024
            }
            .values(),
            ("—".into(), "1 KiB".into())
        );
    }

    #[test]
    fn dropping_stats_wakes_and_stops_worker_without_waiting_for_next_sample() {
        let (sampled, receiver) = mpsc::channel();
        let (stats, worker) = ProcessStats::spawn(
            move || {
                let _ = sampled.send(());
                Some(Snapshot {
                    cpu: Some(200.0),
                    resident: 1024,
                })
            },
            Duration::from_secs(60),
        )
        .unwrap();
        receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        let weak = Arc::downgrade(&stats.latest);
        drop(stats);
        worker.join().unwrap();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn os_sampler_reads_this_process_resident_memory() {
        if !sysinfo::IS_SUPPORTED_SYSTEM {
            return;
        }
        let stats = ProcessStats::start();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(snapshot) = stats.latest.lock().unwrap().as_ref() {
                assert!(snapshot.resident > 0);
                assert!(snapshot.cpu.is_none_or(|cpu| cpu.is_finite() && cpu >= 0.0));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "process stats were unavailable"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
