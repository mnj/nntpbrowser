use super::*;

#[test]
fn servers_groups_and_file_mode() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    #[cfg(unix)]
    {
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    let id = catalog.insert_server(&server("main")).unwrap();
    catalog.set_header_compression(id, "xzver").unwrap();
    let listed = catalog.list_servers().unwrap();
    assert_eq!(listed[0].password.as_deref(), Some("s3cret-pass"));
    assert_eq!(listed[0].header_compression, "xzver");
    let gid = catalog.subscribe(id, "alt.binaries.test").unwrap();
    catalog.set_group_marks(gid, 1, 100, 40, Some(10)).unwrap();
    catalog.unsubscribe(id, "alt.binaries.test").unwrap();
    let groups = catalog.list_groups(id).unwrap();
    assert!(groups.is_empty());
    let fresh = catalog.subscribe(id, "alt.binaries.test").unwrap();
    let group = catalog.group_by_id(fresh).unwrap().unwrap();
    assert_eq!((group.low, group.high, group.watermark), (0, 0, 0));
    catalog.delete_server(id).unwrap();
    assert!(catalog.list_groups(id).unwrap().is_empty());
    let _ = fs::remove_file(&path);
}

#[test]
fn version_one_migrates_starttls_and_preserves_server_credentials() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let id = catalog.insert_server(&server("legacy")).unwrap();
    drop(catalog);
    let legacy = Connection::open(&path).unwrap();
    legacy
        .execute("ALTER TABLE server DROP COLUMN starttls", [])
        .unwrap();
    legacy.execute("PRAGMA user_version=1", []).unwrap();
    drop(legacy);

    let catalog = Catalog::open(&path).unwrap();
    let row = catalog.server_by_id(id).unwrap().unwrap();
    assert!(row.tls);
    assert!(!row.starttls);
    assert_eq!(row.username.as_deref(), Some("reader"));
    assert_eq!(row.password.as_deref(), Some("s3cret-pass"));
    let version: i64 = catalog
        .lock()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 3);
    let mut settings = server("upgraded");
    settings.tls = false;
    settings.starttls = true;
    catalog.update_server(id, &settings, true).unwrap();
    let row = catalog.server_by_id(id).unwrap().unwrap();
    assert!(!row.tls);
    assert!(row.starttls);
    settings.tls = true;
    assert!(catalog.update_server(id, &settings, true).is_err());
    assert!(catalog.insert_server(&settings).is_err());
    drop(catalog);
    let reopened = Catalog::open(&path).unwrap();
    assert!(reopened.server_by_id(id).unwrap().unwrap().starttls);
    drop(reopened);
    let _ = fs::remove_file(path);
}
