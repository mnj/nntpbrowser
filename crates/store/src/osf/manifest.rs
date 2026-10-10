use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Manifest {
    pub(super) watermark: u64,
    pub(super) files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ManifestFile {
    pub(super) name: String,
    pub(super) low: u64,
    pub(super) high: u64,
}

pub(super) fn read_manifest(dir: &Path) -> Result<Manifest, Error> {
    let path = dir.join("manifest.json");
    if !path.exists() {
        return Ok(Manifest {
            watermark: 0,
            files: Vec::new(),
        });
    }
    let text = fs::read_to_string(path)?;
    let mut manifest: Manifest =
        serde_json::from_str(&text).map_err(|err| Error::Header(err.to_string()))?;
    manifest.files.sort_by_key(|file| file.low);
    if manifest.files.iter().any(|file| {
        file.low > file.high
            || Path::new(&file.name).components().count() != 1
            || !file.name.starts_with("r_")
            || !file.name.ends_with(".osf")
    }) || manifest
        .files
        .windows(2)
        .any(|pair| pair[0].high >= pair[1].low)
    {
        return Err(Error::Header("invalid manifest ranges".into()));
    }
    Ok(manifest)
}

pub(super) fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), Error> {
    let path = dir.join("manifest.json");
    let tmp = dir.join("manifest.json.tmp");
    {
        let mut file = create_file(&tmp)?;
        let body =
            serde_json::to_vec_pretty(manifest).map_err(|err| Error::Header(err.to_string()))?;
        file.write_all(&body)?;
        file.sync_all()?;
    }
    set_file_mode(&tmp)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}
