use super::security::{caps, line, send};
use super::*;
use crate::sasl;
use ring::{digest, hmac, pbkdf2};
use std::num::NonZeroU32;

#[tokio::test]
async fn scram_checks_both_server_proof_endings_and_rejects_tampering() {
    for (ending, tamper) in [(283, false), (383, false), (283, true)] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = plain_cfg(listener.local_addr().unwrap());
        cfg.username = Some("user".into());
        cfg.password = Some("pencil".into());
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            send(&mut socket, "201 ready\r\n").await;
            caps(&mut socket, "SASL PLAIN SCRAM-SHA-256\r\n").await;
            let first = line(&mut socket).await;
            let first = String::from_utf8(
                sasl::decode(first.strip_prefix("AUTHINFO SASL SCRAM-SHA-256 ").unwrap()).unwrap(),
            )
            .unwrap();
            let bare = first.strip_prefix("n,,").unwrap();
            let nonce = bare.strip_prefix("n=user,r=").unwrap();
            let extended = format!("{nonce}-server-nonce");
            let challenge = format!("r={extended},s=c2FsdA==,i=4096");
            send(
                &mut socket,
                &format!("383 {}\r\n", sasl::encode(challenge.as_bytes())),
            )
            .await;
            let final_message =
                String::from_utf8(sasl::decode(&line(&mut socket).await).unwrap()).unwrap();
            let (without_proof, proof) = final_message.rsplit_once(",p=").unwrap();
            assert_eq!(without_proof, format!("c=biws,r={extended}"));
            let auth = format!("{bare},{challenge},{without_proof}");
            // Independent server-side calculation verifies the submitted client proof.
            let mut salted = [0u8; 32];
            pbkdf2::derive(
                pbkdf2::PBKDF2_HMAC_SHA256,
                NonZeroU32::new(4096).unwrap(),
                b"salt",
                b"pencil",
                &mut salted,
            );
            let key = hmac::Key::new(hmac::HMAC_SHA256, &salted);
            let client_key = hmac::sign(&key, b"Client Key");
            let stored = digest::digest(&digest::SHA256, client_key.as_ref());
            let signature = hmac::sign(
                &hmac::Key::new(hmac::HMAC_SHA256, stored.as_ref()),
                auth.as_bytes(),
            );
            let expected: Vec<u8> = client_key
                .as_ref()
                .iter()
                .zip(signature.as_ref())
                .map(|(a, b)| a ^ b)
                .collect();
            assert_eq!(sasl::decode(proof).unwrap(), expected);
            let server_key = hmac::sign(&key, b"Server Key");
            let signature = hmac::sign(
                &hmac::Key::new(hmac::HMAC_SHA256, server_key.as_ref()),
                auth.as_bytes(),
            );
            let proof = if tamper {
                vec![0; 32]
            } else {
                signature.as_ref().to_vec()
            };
            send(
                &mut socket,
                &format!(
                    "{ending} {}\r\n",
                    sasl::encode(format!("v={}", sasl::encode(&proof)).as_bytes())
                ),
            )
            .await;
            if tamper {
                // Neither weaker SASL PLAIN nor USER/PASS may follow a bad proof.
                assert_eq!(socket.read(&mut [0u8; 1]).await.unwrap(), 0);
            } else {
                if ending == 383 {
                    assert_eq!(line(&mut socket).await, "=");
                    send(&mut socket, "281 authenticated\r\n").await;
                }
                caps(&mut socket, "OVER\r\n").await;
            }
        });
        let result = Connection::connect(&cfg).await;
        if tamper {
            assert!(matches!(result, Err(Error::Auth)));
        } else {
            assert!(result.is_ok());
        }
        peer.await.unwrap();
    }
}

#[tokio::test]
async fn advertised_unavailable_sasl_can_fall_back_but_bad_password_cannot() {
    for unavailable in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = plain_cfg(listener.local_addr().unwrap());
        cfg.username = Some("user".into());
        cfg.password = Some("secret".into());
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            send(&mut socket, "201 ready\r\n").await;
            caps(&mut socket, "SASL PLAIN\r\nAUTHINFO USER\r\n").await;
            assert!(line(&mut socket).await.starts_with("AUTHINFO SASL PLAIN "));
            if unavailable {
                send(&mut socket, "503 mechanism unavailable\r\n").await;
                assert_eq!(line(&mut socket).await, "AUTHINFO USER user");
                send(&mut socket, "381 password needed\r\n").await;
                assert_eq!(line(&mut socket).await, "AUTHINFO PASS secret");
                send(&mut socket, "281 authenticated\r\n").await;
                caps(&mut socket, "OVER\r\n").await;
            } else {
                send(&mut socket, "481 wrong password: secret\r\n").await;
                assert_eq!(socket.read(&mut [0u8; 1]).await.unwrap(), 0);
            }
        });
        let result = Connection::connect(&cfg).await;
        if unavailable {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(Error::Auth)));
        }
        peer.await.unwrap();
    }
}
