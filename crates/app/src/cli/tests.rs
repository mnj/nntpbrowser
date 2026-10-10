use super::*;

fn parse(args: &[&str]) -> Result<Option<CliCommand>, String> {
    parse_cli(&args.iter().map(|arg| (*arg).into()).collect::<Vec<_>>())
}

#[test]
fn no_arguments_launches_gui_and_help_stays_headless() {
    assert!(matches!(parse(&[]), Ok(None)));
    for help in ["help", "--help", "-h"] {
        assert!(matches!(parse(&[help]), Ok(Some(CliCommand::Help))));
    }
}

#[test]
fn server_defaults_and_explicit_options() {
    let Ok(Some(CliCommand::ServerAdd { server, enabled })) =
        parse(&["server-add", "--name", "Primary", "--host", "news.example"])
    else {
        panic!("server args");
    };
    assert_eq!(
        (
            server.port,
            server.tls,
            server.priority,
            server.max_connections,
            server.pipeline_depth
        ),
        (563, true, 1, 8, 8)
    );
    assert!(enabled);
    assert!(server.password.is_none());
    let Ok(Some(CliCommand::ServerAdd { server, enabled })) = parse(&[
        "server-add",
        "--name",
        "Local",
        "--host",
        "localhost",
        "--plain",
        "--fill",
        "--disabled",
        "--user",
        "reader",
        "--password",
        "secret",
        "--priority",
        "-2",
        "--connections",
        "4",
        "--pipeline",
        "16",
    ]) else {
        panic!("server args");
    };
    assert_eq!(server.port, 119);
    assert!(!server.tls && server.fill && !enabled);
    assert_eq!(server.password.as_deref(), Some("secret"));
    assert_eq!(server.priority, -2);
    let Ok(Some(CliCommand::ServerAdd { server, .. })) = parse(&[
        "server-add",
        "--name",
        "Local",
        "--host",
        "localhost",
        "--plain",
        "--port",
        "8119",
    ]) else {
        panic!("server args");
    };
    assert_eq!(server.port, 8119);
}

#[test]
fn starttls_uses_port_119_and_rejects_plain_conflicts() {
    let Ok(Some(CliCommand::ServerAdd { server, .. })) = parse(&[
        "server-add",
        "--name",
        "Secure",
        "--host",
        "localhost",
        "--starttls",
    ]) else {
        panic!("STARTTLS args");
    };
    assert!(server.starttls && !server.tls);
    assert_eq!(server.port, 119);
    for flags in [["--plain", "--starttls"], ["--starttls", "--plain"]] {
        assert!(parse(&[
            "server-add",
            "--name",
            "Secure",
            "--host",
            "localhost",
            flags[0],
            flags[1]
        ])
        .is_err());
    }
}

#[test]
fn all_id_commands_parse_without_a_server() {
    for name in [
        "server-test",
        "server-delete",
        "open",
        "queue",
        "pause",
        "resume",
        "cancel",
        "preview",
    ] {
        assert!(parse(&[name, "42"]).unwrap().is_some());
        for id in ["0", "-1", "no", "9223372036854775808"] {
            assert!(parse(&[name, id]).is_err());
        }
        assert!(parse(&[name, "42", "unexpected"]).is_err());
    }
    for name in ["servers", "jobs"] {
        assert!(parse(&[name]).unwrap().is_some());
        assert!(parse(&[name, "unexpected"]).is_err());
    }
}

#[test]
fn group_post_and_header_commands() {
    assert!(
        matches!(parse(&["groups", "1"]), Ok(Some(CliCommand::Groups { wildmat, .. })) if wildmat == "*")
    );
    assert!(
        matches!(parse(&["groups", "1", "--wildmat", "alt.binaries.*"]), Ok(Some(CliCommand::Groups { wildmat, .. })) if wildmat == "alt.binaries.*")
    );
    assert!(
        matches!(parse(&["posts", "3", "--search", "hello world"]), Ok(Some(CliCommand::Posts { search, .. })) if search == "hello world")
    );
    assert!(matches!(
        parse(&["sync", "3"]),
        Ok(Some(CliCommand::Sync { limit: None, .. }))
    ));
    assert!(matches!(
        parse(&["sync", "3", "--limit", "1000"]),
        Ok(Some(CliCommand::Sync {
            limit: Some(Some(1000)),
            ..
        }))
    ));
    assert!(matches!(
        parse(&["sync", "3", "--all"]),
        Ok(Some(CliCommand::Sync {
            limit: Some(None),
            ..
        }))
    ));
    for name in ["subscribe", "unsubscribe"] {
        assert!(parse(&[name, "1", "alt.binaries.test"]).is_ok());
        assert!(parse(&[name, "1", "bad name"]).is_err());
    }
    assert!(matches!(
        parse(&["nzb", "/tmp/example.nzb"]),
        Ok(Some(CliCommand::Nzb(_)))
    ));
}

#[test]
fn invalid_options_are_rejected_without_echoing_secrets() {
    for args in [
        vec!["server-add", "--password", "secret"],
        vec![
            "server-add",
            "--name",
            "Test",
            "--host",
            "test",
            "--port",
            "0",
        ],
        vec![
            "server-add",
            "--name",
            "Test",
            "--host",
            "test",
            "--connections",
            "0",
        ],
        vec![
            "server-add",
            "--name",
            "Test",
            "--host",
            "test",
            "--host",
            "other",
        ],
        vec!["sync", "1", "--limit", "0"],
        vec!["groups", "1", "--unknown", "secret"],
        vec!["secret"],
    ] {
        let error = parse(&args).err().expect("reject malformed arguments");
        assert!(!error.contains("secret"));
    }
}
