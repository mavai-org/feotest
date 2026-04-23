//! Integration tests that drive the `sentinel_demo` example through the
//! SN03 sink plumbing.
//!
//! These tests complement `tests/sentinel_demo.rs` by focusing on the
//! verdict-sink surface specifically: the default console sink's output
//! shape, and the webhook sink's end-to-end behaviour when
//! `FEOTEST_WEBHOOK_URL` is set.

#![cfg(feature = "webhook")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn sentinel(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO"));
    cmd.args([
        "run",
        "--quiet",
        "--features",
        "webhook",
        "--example",
        "sentinel_demo",
        "--",
    ]);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn sentinel_demo")
}

#[derive(Debug, Default)]
struct CapturedRequest {
    method: String,
    headers: Vec<(String, String)>,
    body: String,
}

fn spawn_one_shot_listener() -> (String, mpsc::Receiver<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        // Accept verdicts until the sentinel run completes. The test only
        // cares that *at least one* arrived, so we forward each parsed
        // request over the channel and keep going until the socket closes.
        while let Ok((mut stream, _)) = listener.accept() {
            let captured = read_request(&stream);
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = stream.flush();
            if tx.send(captured).is_err() {
                break;
            }
        }
    });
    (format!("http://{addr}/verdicts"), rx)
}

fn read_request(stream: &std::net::TcpStream) -> CapturedRequest {
    let mut captured = CapturedRequest::default();
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut first = String::new();
    reader.read_line(&mut first).expect("request line");
    captured.method = first.split_whitespace().next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("header line");
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or(0);
            }
            captured.headers.push((name, value));
        }
    }

    let mut body_bytes = vec![0u8; content_length];
    reader.read_exact(&mut body_bytes).expect("read body");
    captured.body = String::from_utf8(body_bytes).expect("utf8 body");
    captured
}

#[test]
fn default_run_still_emits_verdict_line_on_stdout() {
    let out = sentinel(&["run", "sla_demo"], &[]);
    assert!(
        out.status.success(),
        "expected default run to succeed: {out:?}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("sla_demo.always_ok"),
        "default console sink must still write a verdict line: {stdout}"
    );
    assert!(
        stdout.contains("\tPass"),
        "default console sink must still include the verdict word: {stdout}"
    );
}

#[test]
fn webhook_url_env_triggers_json_post() {
    let (url, rx) = spawn_one_shot_listener();

    let out = sentinel(&["run", "sla_demo"], &[("FEOTEST_WEBHOOK_URL", &url)]);
    assert!(
        out.status.success(),
        "run with webhook configured should still exit 0: {out:?}"
    );

    let captured = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("webhook listener received a request");
    assert_eq!(captured.method, "POST");

    let content_type = captured
        .headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-type"))
        .map_or("", |(_, v)| v.as_str());
    assert!(
        content_type.starts_with("application/json"),
        "expected application/json content type: {content_type}"
    );

    let body: serde_json::Value = serde_json::from_str(&captured.body).expect("body is json");
    assert_eq!(body["verdict"], "PASS");
    assert_eq!(body["identity"]["useCaseId"], "sla_demo.always_ok");
}
