//! Local peers exercise TLS state transitions and SASL wire exchanges.
use super::*;
use crate::sasl;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsAcceptor;

pub(super) async fn line<S: AsyncRead + Unpin>(stream: &mut S) -> String {
    let mut bytes = Vec::new();
    loop {
        let byte = tokio::time::timeout(Duration::from_secs(2), stream.read_u8())
            .await
            .unwrap()
            .unwrap();
        bytes.push(byte);
        assert!(bytes.len() <= sasl::WIRE_LIMIT);
        if byte == b'\n' {
            break;
        }
    }
    String::from_utf8(bytes)
        .unwrap()
        .trim_end_matches(['\r', '\n'])
        .into()
}

pub(super) async fn send<S: AsyncWrite + Unpin>(stream: &mut S, text: &str) {
    stream.write_all(text.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();
}

pub(super) async fn caps<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, extra: &str) {
    assert_eq!(line(stream).await, "CAPABILITIES");
    send(
        stream,
        &format!("101 capabilities\r\nVERSION 2\r\nREADER\r\n{extra}.\r\n"),
    )
    .await;
}

fn policies() -> (TlsAcceptor, Arc<rustls::ClientConfig>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der())),
        )
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.der().clone()).unwrap();
    let client = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    (TlsAcceptor::from(Arc::new(server)), Arc::new(client))
}

#[tokio::test]
async fn starttls_refreshes_capabilities_and_authenticates_only_inside_tls() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (acceptor, trust) = policies();
    let mut cfg = plain_cfg(listener.local_addr().unwrap());
    cfg.host = "localhost".into();
    cfg.tls = TlsMode::StartTls;
    cfg.username = Some("user".into());
    cfg.password = Some("secret".into());
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        send(&mut socket, "201 ready\r\n").await;
        caps(
            &mut socket,
            "STARTTLS\r\nCOMPRESS DEFLATE\r\nMODE-READER\r\n",
        )
        .await;
        assert_eq!(line(&mut socket).await, "STARTTLS");
        send(&mut socket, "382 continue with TLS\r\n").await;
        let mut socket = acceptor.accept(socket).await.unwrap();
        caps(&mut socket, "SASL PLAIN\r\n").await;
        let auth = line(&mut socket).await;
        let payload = auth.strip_prefix("AUTHINFO SASL PLAIN ").unwrap();
        assert_eq!(sasl::decode(payload).unwrap(), b"\0user\0secret");
        send(&mut socket, "281 authenticated\r\n").await;
        // Deliberately includes MODE-READER: it remains prohibited under TLS.
        caps(&mut socket, "OVER\r\nMODE-READER\r\n").await;
        assert_eq!(line(&mut socket).await, "DATE");
        send(&mut socket, "111 20261010120000\r\n").await;
    });
    let mut connection = Connection::connect_with_tls_config(&cfg, trust)
        .await
        .unwrap();
    assert!(!connection
        .capabilities()
        .entries
        .iter()
        .any(|(name, _)| name == "COMPRESS"));
    assert_eq!(connection.date().await.unwrap(), "20261010120000");
    peer.await.unwrap();
}

#[tokio::test]
async fn required_starttls_never_falls_back_to_plaintext_authentication() {
    for refusal in [None, Some("580 unavailable\r\n")] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = plain_cfg(listener.local_addr().unwrap());
        cfg.tls = TlsMode::StartTls;
        cfg.username = Some("user".into());
        cfg.password = Some("secret".into());
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            send(&mut socket, "201 ready\r\n").await;
            caps(
                &mut socket,
                if refusal.is_some() {
                    "STARTTLS\r\n"
                } else {
                    "AUTHINFO USER\r\n"
                },
            )
            .await;
            if let Some(refusal) = refusal {
                assert_eq!(line(&mut socket).await, "STARTTLS");
                send(&mut socket, refusal).await;
            }
            assert_eq!(socket.read(&mut [0u8; 1]).await.unwrap(), 0);
        });
        assert!(matches!(
            Connection::connect(&cfg).await,
            Err(Error::Tls(_))
        ));
        peer.await.unwrap();
    }
}

#[tokio::test]
async fn tls_validates_certificate_trust_and_hostname_in_both_modes() {
    for mode in [TlsMode::Implicit, TlsMode::StartTls] {
        for trusted in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let (acceptor, trust) = policies();
            // Certificate is for localhost, so even the trusted root fails this IP name.
            let mut cfg = plain_cfg(listener.local_addr().unwrap());
            cfg.tls = mode;
            let peer = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                if mode == TlsMode::StartTls {
                    send(&mut socket, "201 ready\r\n").await;
                    caps(&mut socket, "STARTTLS\r\n").await;
                    assert_eq!(line(&mut socket).await, "STARTTLS");
                    send(&mut socket, "382 continue\r\n").await;
                }
                assert!(acceptor.accept(socket).await.is_err());
            });
            let result = if trusted {
                Connection::connect_with_tls_config(&cfg, trust).await
            } else {
                Connection::connect(&cfg).await
            };
            assert!(matches!(result, Err(Error::Tls(_))));
            peer.await.unwrap();
        }
    }
}

#[tokio::test]
async fn sasl_plain_supports_long_responses_and_empty_challenge() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = plain_cfg(listener.local_addr().unwrap());
    cfg.username = Some("u".repeat(255));
    cfg.password = Some("p".repeat(255));
    let expected = sasl::plain(
        cfg.username.as_ref().unwrap(),
        cfg.password.as_ref().unwrap(),
    )
    .unwrap();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        send(&mut socket, "201 ready\r\n").await;
        caps(&mut socket, "SASL PLAIN\r\n").await;
        let first = line(&mut socket).await;
        assert!(first.len() > 512);
        assert_eq!(
            sasl::decode(first.strip_prefix("AUTHINFO SASL PLAIN ").unwrap()).unwrap(),
            expected
        );
        send(&mut socket, "383 =\r\n").await;
        assert_eq!(sasl::decode(&line(&mut socket).await).unwrap(), expected);
        send(&mut socket, "281 success\r\n").await;
        caps(&mut socket, "OVER\r\n").await;
    });
    Connection::connect(&cfg).await.unwrap();
    peer.await.unwrap();
}

#[tokio::test]
async fn malformed_sasl_challenge_is_cancelled_without_credential_fallback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = plain_cfg(listener.local_addr().unwrap());
    cfg.username = Some("user".into());
    cfg.password = Some("private-password".into());
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        send(&mut socket, "201 ready\r\n").await;
        caps(&mut socket, "SASL SCRAM-SHA-256 PLAIN\r\n").await;
        assert!(line(&mut socket)
            .await
            .starts_with("AUTHINFO SASL SCRAM-SHA-256 "));
        send(&mut socket, "383 invalid-base64!\r\n").await;
        assert_eq!(line(&mut socket).await, "*");
        send(&mut socket, "481 cancelled\r\n").await;
        assert_eq!(socket.read(&mut [0u8; 1]).await.unwrap(), 0);
    });
    let error = Connection::connect(&cfg).await.err().unwrap();
    assert!(matches!(error, Error::Auth));
    assert!(!error.to_string().contains("private-password"));
    peer.await.unwrap();
}
