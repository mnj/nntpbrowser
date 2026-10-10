//! Strict conventional recovery-volume names; sizes are only scheduling hints.

pub(super) fn volume_range(name: &str) -> Option<(u64, u64)> {
    store::recovery_volume_range(name)
}
