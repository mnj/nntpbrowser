use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flate2::{Compress, Compression, Decompress};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::codec::pop_line;
use crate::compress::{compress_sync, inflate_pending};
use crate::conn::Connection;
use crate::error::Error;
use crate::inflate::{xz_compress, zlib_compress};
use crate::pool::Farm;
use crate::types::{ArticleRef, Config, HeaderStrategy, ServerEndpoint, TlsMode};
use crate::yenc;

fn plain_cfg(addr: SocketAddr) -> Config {
    Config {
        host: "127.0.0.1".into(),
        port: addr.port(),
        tls: TlsMode::Plain,
        username: None,
        password: None,
        timeout: Duration::from_secs(2),
    }
}

fn endpoint(id: i64, addr: SocketAddr, priority: i32, fill: bool) -> ServerEndpoint {
    ServerEndpoint {
        id,
        config: plain_cfg(addr),
        priority,
        fill,
        max_connections: 2,
        pipeline_depth: 4,
    }
}

async fn serve<F, Fut>(handler: F) -> SocketAddr
where
    F: Fn(TcpStream) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(handler(sock));
        }
    });
    tokio::task::yield_now().await;
    addr
}

async fn read_line(sock: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(2), sock.read(&mut byte))
            .await
            .expect("read timeout")
            .expect("read");
        if n == 0 {
            panic!("eof before newline: {}", String::from_utf8_lossy(&buf));
        }
        buf.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    String::from_utf8_lossy(&buf)
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

async fn write_all(sock: &mut TcpStream, data: &[u8]) {
    sock.write_all(data).await.unwrap();
    sock.flush().await.unwrap();
}

async fn greet(sock: &mut TcpStream) {
    write_all(sock, b"201 reader ready\r\n").await;
}

async fn capabilities(sock: &mut TcpStream, extra: &str) {
    let cmd = read_line(sock).await;
    assert!(cmd.eq_ignore_ascii_case("CAPABILITIES"), "{cmd}");
    let body = format!("101 Capabilities\r\nVERSION 2\r\nREADER\r\nOVER\r\n{extra}.\r\n");
    write_all(sock, body.as_bytes()).await;
}

async fn overview_fmt(sock: &mut TcpStream) {
    let cmd = read_line(sock).await;
    assert!(cmd.starts_with("LIST OVERVIEW.FMT"), "{cmd}");
    write_all(
        sock,
        b"215 fields\r\nSubject:\r\nFrom:\r\nDate:\r\nMessage-ID:\r\nReferences:\r\n:bytes\r\n:lines\r\n.\r\n",
    )
    .await;
}

fn stuff(line: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if line.first() == Some(&b'.') {
        out.push(b'.');
    }
    out.extend_from_slice(line);
    out.extend_from_slice(b"\r\n");
    out
}

async fn write_multiline(sock: &mut TcpStream, status: &str, lines: &[Vec<u8>]) {
    let mut out = format!("{status}\r\n").into_bytes();
    for line in lines {
        out.extend(stuff(line));
    }
    out.extend_from_slice(b".\r\n");
    write_all(sock, &out).await;
}

mod commands;
mod compression;
mod farm;
mod streaming;

mod provider;
mod reader;

mod security;

mod sasl_exchange;

mod body_budget;
mod connection_stats;
mod traffic;
