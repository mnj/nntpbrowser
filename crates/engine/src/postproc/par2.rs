//! Local recovery only: no header discovery or network recovery-volume fetching.

use super::PostError;
mod coverage;
pub(crate) use coverage::{volume_covers_range, volume_overlaps_range};
use std::{collections::HashSet, path::Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairReport {
    NoPar2,
    Clean,
    Repaired,
    Insufficient(String),
}

#[derive(Debug, Clone)]
pub(crate) struct Par2Inspection {
    pub set_id: String,
    pub protected_names: Vec<String>,
    pub blocks_needed: u32,
    pub blocks_available: u32,
    pub clean: bool,
}

pub(crate) fn inspect_dir_checked(
    dir: &Path,
    required: &[String],
    pinned_set: Option<&str>,
) -> Result<Option<Par2Inspection>, PostError> {
    let Some(set) = select_set(dir, required, pinned_set)? else {
        return Ok(None);
    };
    let verified = rust_par2::verify(&set, dir);
    let mut protected_names: Vec<_> = set
        .files
        .values()
        .map(|file| file.filename.clone())
        .collect();
    protected_names.sort();
    Ok(Some(Par2Inspection {
        set_id: set_id(&set),
        protected_names,
        blocks_needed: verified.blocks_needed(),
        blocks_available: verified.recovery_blocks_available,
        clean: verified.all_correct(),
    }))
}

fn set_id(set: &rust_par2::types::Par2FileSet) -> String {
    set.recovery_set_id
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Verify and repair the local PAR2 set. Recovery volumes must already be on disk.
pub fn repair_dir(dir: &Path) -> Result<RepairReport, PostError> {
    repair_dir_checked(dir, &[])
}

/// Unresolved downloads can complete only if this set protects those exact files.
pub(crate) fn repair_dir_checked(
    dir: &Path,
    required: &[String],
) -> Result<RepairReport, PostError> {
    repair_dir_pinned(dir, required, None)
}

pub(crate) fn repair_dir_pinned(
    dir: &Path,
    required: &[String],
    pinned_set: Option<&str>,
) -> Result<RepairReport, PostError> {
    let Some(set) = select_set(dir, required, pinned_set)? else {
        return Ok(RepairReport::NoPar2);
    };
    let verified = rust_par2::verify(&set, dir);
    if verified.all_correct() {
        return Ok(RepairReport::Clean);
    }
    if !verified.repair_possible {
        return Ok(RepairReport::Insufficient(format!(
            "need {} recovery blocks, have {}",
            verified.blocks_needed(),
            verified.recovery_blocks_available
        )));
    }
    let repaired = rust_par2::repair::repair_from_verify(&set, dir, &verified)
        .map_err(|err| PostError::Par2(err.to_string()))?;
    if !repaired.success || !rust_par2::verify(&set, dir).all_correct() {
        return Err(PostError::Par2("repaired files failed verification".into()));
    }
    Ok(RepairReport::Repaired)
}

fn select_set(
    dir: &Path,
    required: &[String],
    pinned_set: Option<&str>,
) -> Result<Option<rust_par2::types::Par2FileSet>, PostError> {
    if !std::fs::symlink_metadata(dir)?.file_type().is_dir() {
        return Err(PostError::Par2(
            "repair target must be a regular download directory".into(),
        ));
    }
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("par2"))
        {
            // The library rescans every local volume; silently skipping a link
            // here would still let it follow that link during verification.
            if !entry.file_type()?.is_file() {
                return Err(PostError::Par2(
                    "PAR2 recovery files must be regular local files".into(),
                ));
            }
            candidates.push(entry.path());
        }
    }
    candidates.sort_by_key(|path| {
        let name = path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase();
        (name.contains(".vol"), name)
    });
    if candidates.is_empty() {
        return Ok(None);
    }
    let mut rejected = "no usable PAR2 metadata".to_string();
    let mut selected = None;
    for path in candidates {
        let set = match rust_par2::parse(&path) {
            Ok(set) => set,
            Err(error) => {
                rejected = error.to_string();
                continue;
            }
        };
        if let Err(error) = validate_set(&set, dir) {
            rejected = error;
            continue;
        }
        if pinned_set.is_some_and(|pinned| pinned != set_id(&set)) {
            rejected = "PAR2 metadata does not match the selected recovery set".into();
            continue;
        }
        if let Some(name) = required
            .iter()
            .find(|name| !set.files.values().any(|file| &file.filename == *name))
        {
            rejected = format!("PAR2 set does not protect unresolved file {name}");
            continue;
        }
        selected = Some(set);
        break;
    }
    let Some(set) = selected else {
        return Err(PostError::Par2(rejected));
    };
    Ok(Some(set))
}

fn validate_set(set: &rust_par2::types::Par2FileSet, dir: &Path) -> Result<(), String> {
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    if set.slice_size == 0 || set.files.is_empty() || set.file_order.len() != set.files.len() {
        return Err("incomplete PAR2 file metadata".into());
    }
    for id in &set.file_order {
        if !ids.insert(id) || !set.files.contains_key(id) {
            return Err("invalid PAR2 source-file order".into());
        }
    }
    for file in set.files.values() {
        let name = &file.filename;
        if name.is_empty()
            || matches!(name.as_str(), "." | "..")
            || name.contains(['/', '\\', ':'])
            || name.chars().any(char::is_control)
            || !names.insert(name)
        {
            return Err("PAR2 filename must be a unique file in the download folder".into());
        }
        if std::fs::symlink_metadata(dir.join(name)).is_ok_and(|meta| !meta.file_type().is_file()) {
            return Err("PAR2 target is not a regular file".into());
        }
    }
    Ok(())
}
