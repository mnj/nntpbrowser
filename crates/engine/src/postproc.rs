//! PAR2 verify/repair and archive extraction after a job's segments are on disk.

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

mod par2;
pub(crate) use par2::{
    inspect_dir_checked, repair_dir_pinned, volume_covers_range, volume_overlaps_range,
    Par2Inspection,
};
pub use par2::{repair_dir, RepairReport};

#[derive(Debug, Error)]
pub enum PostError {
    #[error("par2: {0}")]
    Par2(String),
    #[error("extract: {0}")]
    Extract(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractReport {
    Unpacked,
    /// RAR set, and neither unrar nor unar is installed.
    PackedRar,
    NothingToUnpack,
}

/// Unpack zip and 7z in-process. RAR only when `unrar` or `unar` is on `PATH`.
/// `password` is forwarded to 7z and to unrar (`-p`). Zip uses the traditional password API.
pub fn extract_archives(dir: &Path, password: Option<&str>) -> Result<ExtractReport, PostError> {
    let mut unpacked = false;
    let mut packed_rar = false;
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();

    for path in entries {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "zip" => {
                extract_zip(&path, dir, password)?;
                unpacked = true;
            }
            "7z" => {
                extract_7z(&path, dir, password)?;
                unpacked = true;
            }
            "rar" => match extract_rar(&path, dir, password)? {
                RarOutcome::Unpacked => unpacked = true,
                RarOutcome::MissingTool => packed_rar = true,
            },
            _ => {}
        }
    }
    if unpacked {
        Ok(ExtractReport::Unpacked)
    } else if packed_rar {
        Ok(ExtractReport::PackedRar)
    } else {
        Ok(ExtractReport::NothingToUnpack)
    }
}

fn extract_zip(path: &Path, dir: &Path, password: Option<&str>) -> Result<(), PostError> {
    let file = std::fs::File::open(path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|err| PostError::Extract(err.to_string()))?;
    for i in 0..archive.len() {
        let mut entry = if let Some(password) = password {
            archive.by_index_decrypt(i, password.as_bytes())
        } else {
            archive.by_index(i)
        }
        .map_err(|err| PostError::Extract(err.to_string()))?;
        let name = entry
            .enclosed_name()
            .ok_or_else(|| PostError::Extract("zip entry escaped the directory".into()))?
            .to_owned();
        let dest = dir.join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}

fn extract_7z(path: &Path, dir: &Path, password: Option<&str>) -> Result<(), PostError> {
    let result = if let Some(password) = password {
        sevenz_rust2::decompress_file_with_password(path, dir, password.into())
    } else {
        sevenz_rust2::decompress_file(path, dir)
    };
    result.map_err(|err| PostError::Extract(err.to_string()))
}

enum RarOutcome {
    Unpacked,
    MissingTool,
}

fn extract_rar(path: &Path, dir: &Path, password: Option<&str>) -> Result<RarOutcome, PostError> {
    if which("unrar") {
        let mut cmd = Command::new("unrar");
        hide_console(&mut cmd);
        cmd.arg("x").arg("-o+").arg("-y");
        if let Some(password) = password {
            cmd.arg(format!("-p{password}"));
        } else {
            cmd.arg("-p-");
        }
        cmd.arg(path).arg(dir);
        let status = cmd.status()?;
        if status.success() {
            return Ok(RarOutcome::Unpacked);
        }
        return Err(PostError::Extract(format!("unrar exited {status}")));
    }
    if which("unar") {
        let mut cmd = Command::new("unar");
        hide_console(&mut cmd);
        cmd.arg("-f").arg("-o").arg(dir);
        if let Some(password) = password {
            cmd.arg("-p").arg(password);
        }
        cmd.arg(path);
        let status = cmd.status()?;
        if status.success() {
            return Ok(RarOutcome::Unpacked);
        }
        return Err(PostError::Extract(format!("unar exited {status}")));
    }
    Ok(RarOutcome::MissingTool)
}

fn which(bin: &str) -> bool {
    #[cfg(target_os = "windows")]
    let mut command = Command::new("where.exe");
    #[cfg(not(target_os = "windows"))]
    let mut command = Command::new("which");
    hide_console(&mut command);
    command
        .arg(bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn hide_console(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}
