//! Webhook verdict sink: blocking HTTP POST per verdict.
//!
//! Feature-gated behind the `webhook` Cargo feature. The default build
//! does not compile this module and does not pull in `ureq`.
//!
//! The sink posts each verdict as JSON (`Content-Type:
//! application/json`). Transport errors and non-2xx responses are
//! captured as [`SinkError::DeliveryFailed`] — the composite sink
//! logs them and the run continues. The sink attempts delivery once
//! per verdict; retry/back-off is deliberately out of scope for the verdict sinks.

use std::time::Duration;

use crate::sentinel::sinks::{SinkError, VerdictSink};
use crate::verdict::VerdictRecord;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// POSTs every verdict as JSON to the configured HTTP endpoint.
///
/// Construct via [`WebhookVerdictSink::builder`] — the builder takes the
/// endpoint URL (required) and lets you attach headers and a non-default
/// timeout. The agent itself is owned by the sink; each `accept` uses it
/// to issue one synchronous POST.
pub struct WebhookVerdictSink {
    endpoint: String,
    headers: Vec<(String, String)>,
    timeout: Duration,
    agent: ureq::Agent,
}

impl WebhookVerdictSink {
    /// Starts a builder for a webhook sink targeting `endpoint`.
    #[must_use]
    pub fn builder(endpoint: impl Into<String>) -> WebhookVerdictSinkBuilder {
        WebhookVerdictSinkBuilder {
            endpoint: endpoint.into(),
            headers: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// The endpoint this sink POSTs to.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

/// Builder for [`WebhookVerdictSink`].
pub struct WebhookVerdictSinkBuilder {
    endpoint: String,
    headers: Vec<(String, String)>,
    timeout: Duration,
}

impl WebhookVerdictSinkBuilder {
    /// Adds a header to every outbound POST.
    ///
    /// Call repeatedly to attach multiple headers (e.g. authentication
    /// token plus a custom trace header).
    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Overrides the per-request timeout. Defaults to 5 seconds.
    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Produces a ready-to-use sink.
    #[must_use]
    pub fn build(self) -> WebhookVerdictSink {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(self.timeout)
            .timeout_read(self.timeout)
            .timeout_write(self.timeout)
            .build();
        WebhookVerdictSink {
            endpoint: self.endpoint,
            headers: self.headers,
            timeout: self.timeout,
            agent,
        }
    }
}

impl VerdictSink for WebhookVerdictSink {
    fn accept(&mut self, verdict: &VerdictRecord) -> Result<(), SinkError> {
        let body = serde_json::to_value(verdict)
            .map_err(|e| SinkError::DeliveryFailed(format!("json encode failed: {e}")))?;

        let mut request = self.agent.post(&self.endpoint);
        request = request.set("Content-Type", "application/json");
        for (name, value) in &self.headers {
            request = request.set(name, value);
        }

        match request.send_json(body) {
            Ok(response) => {
                let status = response.status();
                if (200..300).contains(&status) {
                    Ok(())
                } else {
                    Err(SinkError::DeliveryFailed(format!(
                        "endpoint {} returned HTTP {status}",
                        self.endpoint
                    )))
                }
            }
            Err(ureq::Error::Status(status, _)) => Err(SinkError::DeliveryFailed(format!(
                "endpoint {} returned HTTP {status}",
                self.endpoint
            ))),
            Err(ureq::Error::Transport(err)) => Err(SinkError::DeliveryFailed(format!(
                "transport error posting to {}: {err}",
                self.endpoint
            ))),
        }
    }
}

impl core::fmt::Debug for WebhookVerdictSink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WebhookVerdictSink")
            .field("endpoint", &self.endpoint)
            .field(
                "headers",
                &self.headers.iter().map(|(k, _)| k).collect::<Vec<_>>(),
            )
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        CostSummary, ExecutionSummary, TerminationInfo, TerminationReason, TestIdentity, TestIntent,
    };
    use crate::verdict::{CriterionRow, FunctionalAssessment, Verdict, VerdictRecord};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    fn sample_verdict() -> VerdictRecord {
        let exec = ExecutionSummary::new(
            100,
            100,
            95,
            5,
            TerminationInfo::new(TerminationReason::Completed),
            CostSummary::new(Duration::from_millis(100), 500, 100),
        );
        VerdictRecord::builder(
            TestIdentity::new("webhook-test").with_test_name("posts"),
            Verdict::Pass,
            TestIntent::Verification,
            exec,
            FunctionalAssessment::single(CriterionRow::result(95, 5, vec![], Verdict::Pass)),
        )
        .build()
    }

    /// Spawns a minimal HTTP listener that accepts one request, captures
    /// its body + headers, replies with the configured `status`, and
    /// exits. Returns the URL to post to and a channel that yields the
    /// captured request once the thread has served it.
    fn spawn_once(status: u16) -> (String, mpsc::Receiver<CapturedRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let captured = read_request(&stream);
            let reason = match status {
                200 => "OK",
                201 => "Created",
                500 => "Internal Server Error",
                _ => "Status",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            let _ = tx.send(captured);
        });
        (format!("http://{addr}/verdicts"), rx)
    }

    #[derive(Debug, Default)]
    struct CapturedRequest {
        method: String,
        headers: Vec<(String, String)>,
        body: String,
    }

    fn read_request(stream: &std::net::TcpStream) -> CapturedRequest {
        let mut captured = CapturedRequest::default();
        // Read headers line-by-line, then read the body based on
        // Content-Length. A BufReader lets us advance line-wise.
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
        let mut first = String::new();
        reader.read_line(&mut first).expect("read request line");
        captured.method = first.split_whitespace().next().unwrap_or("").to_string();

        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read header line");
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
        reader.read_exact(&mut body_bytes).expect("read body bytes");
        captured.body = String::from_utf8(body_bytes).expect("body utf8");
        captured
    }

    #[test]
    fn posts_verdict_as_json_with_configured_headers() {
        let (url, rx) = spawn_once(200);

        let mut sink = WebhookVerdictSink::builder(&url)
            .header("X-Sentinel-Token", "secret")
            .timeout(Duration::from_secs(2))
            .build();

        sink.accept(&sample_verdict()).expect("accept succeeds");

        let captured = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("listener served request");
        assert_eq!(captured.method, "POST");
        let content_type = captured
            .headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("content-type"))
            .map_or("", |(_, v)| v.as_str());
        assert!(
            content_type.starts_with("application/json"),
            "Content-Type missing or wrong: {content_type}"
        );
        let custom_header = captured
            .headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("x-sentinel-token"))
            .map(|(_, v)| v.as_str());
        assert_eq!(custom_header, Some("secret"));

        let value: serde_json::Value = serde_json::from_str(&captured.body).expect("body is json");
        assert_eq!(value["identity"]["useCaseId"], "webhook-test");
        assert_eq!(value["identity"]["testName"], "posts");
        assert_eq!(value["verdict"], "PASS");
    }

    #[test]
    fn non_2xx_response_returns_delivery_failed() {
        let (url, _rx) = spawn_once(500);

        let mut sink = WebhookVerdictSink::builder(&url)
            .timeout(Duration::from_secs(2))
            .build();

        let err = sink
            .accept(&sample_verdict())
            .expect_err("500 response should surface as DeliveryFailed");
        match err {
            SinkError::DeliveryFailed(msg) => {
                assert!(
                    msg.contains("500"),
                    "DeliveryFailed message should mention status: {msg}"
                );
            }
            other => panic!("expected DeliveryFailed, got {other:?}"),
        }
    }

    #[test]
    fn transport_error_returns_delivery_failed() {
        // Bind and close immediately — the port is vacant by the time the
        // sink tries to connect, producing a transport error rather than
        // an HTTP status.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        drop(listener);

        let mut sink = WebhookVerdictSink::builder(format!("http://{addr}/verdicts"))
            .timeout(Duration::from_millis(500))
            .build();

        let err = sink
            .accept(&sample_verdict())
            .expect_err("unreachable endpoint should surface as DeliveryFailed");
        assert!(
            matches!(err, SinkError::DeliveryFailed(_)),
            "expected DeliveryFailed, got {err}"
        );
    }
}
