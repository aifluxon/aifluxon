use serde_json::Value;
use std::time::Duration;

pub const MAX_HTTP_ATTEMPTS: u8 = 3;
pub const MAX_PROVIDER_ERROR_CHARS: usize = 1_200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpClientTuning {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub pool_idle_timeout: Duration,
    pub pool_max_idle_per_host: usize,
    pub http1_only: bool,
}

impl Default for HttpClientTuning {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(180),
            pool_idle_timeout: Duration::from_secs(90),
            pool_max_idle_per_host: 8,
            http1_only: true,
        }
    }
}

pub fn build_http_client(tuning: HttpClientTuning) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .use_rustls_tls()
        .pool_max_idle_per_host(tuning.pool_max_idle_per_host)
        .pool_idle_timeout(tuning.pool_idle_timeout)
        .connect_timeout(tuning.connect_timeout)
        .read_timeout(tuning.read_timeout);
    if tuning.http1_only {
        builder = builder.http1_only();
    }
    builder.build().map_err(|error| error.to_string())
}

pub fn is_transient_reqwest_error(error: &reqwest::Error) -> bool {
    let text = error.to_string().to_ascii_lowercase();
    error.is_timeout()
        || error.is_connect()
        || text.contains("connection closed")
        || text.contains("unexpected eof")
        || text.contains("incomplete message")
        || text.contains("sendrequest")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportFailureKind {
    Timeout,
    Connect,
    ConnectionClosed,
    UnexpectedEof,
    NonTransient,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportFailure {
    pub kind: TransportFailureKind,
    pub message: String,
}

impl TransportFailure {
    pub fn transient(kind: TransportFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn is_transient(&self) -> bool {
        !matches!(self.kind, TransportFailureKind::NonTransient)
    }
}

#[async_trait::async_trait]
pub trait HttpTransport: Send + Sync {
    async fn send(&self, attempt: u8) -> Result<Value, TransportFailure>;
    fn request_is_cloneable(&self) -> bool;
}

pub async fn send_with_retry<T: HttpTransport>(transport: &T) -> Result<Value, TransportFailure> {
    let mut attempt = 1;
    loop {
        match transport.send(attempt).await {
            Ok(value) => return Ok(value),
            Err(error)
                if attempt < MAX_HTTP_ATTEMPTS
                    && transport.request_is_cloneable()
                    && error.is_transient() =>
            {
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn retry_backoff(attempt: u8) -> Duration {
    let base = 300 * (1_u64 << attempt.saturating_sub(1).min(5));
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64
        % 101;
    Duration::from_millis(base + jitter)
}

pub fn is_retryable_http_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

pub fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let seconds = headers
        .get("retry-after-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok())
        .map(|ms| ms / 1000.0)
        .or_else(|| {
            headers
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| {
                    value.parse::<f64>().ok().or_else(|| {
                        httpdate::parse_http_date(value).ok().map(|date| {
                            date.duration_since(std::time::SystemTime::now())
                                .unwrap_or_default()
                                .as_secs_f64()
                        })
                    })
                })
        })?;
    // Preserve excessive server delays as a no-retry signal, including overflow.
    if seconds > 60.0 {
        return Some(Duration::from_secs(61));
    }
    Duration::try_from_secs_f64(seconds).ok()
}

/// Replay only requests whose remote effects may safely be repeated. Stateful
/// operations may retry connection setup failures, before a request was sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpRetryPolicy {
    Replayable,
    ConnectOnly,
}

pub fn generation_retry_policy(body: &Value) -> HttpRetryPolicy {
    let has_hosted_work = body
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool.get("type").and_then(Value::as_str) != Some("function"))
        });
    if has_hosted_work {
        HttpRetryPolicy::ConnectOnly
    } else {
        HttpRetryPolicy::Replayable
    }
}

pub async fn send_http_request(
    context: &str,
    request: reqwest::RequestBuilder,
    secrets: &[&str],
    policy: HttpRetryPolicy,
) -> Result<reqwest::Response, String> {
    let mut request = request;
    for attempt in 1..=MAX_HTTP_ATTEMPTS {
        let next = request.try_clone();
        let result = request.send().await;
        let (retryable, hint) = match &result {
            Ok(response) => (
                policy == HttpRetryPolicy::Replayable
                    && is_retryable_http_status(response.status().as_u16()),
                retry_after(response.headers()),
            ),
            Err(error) => (
                match policy {
                    HttpRetryPolicy::Replayable => is_transient_reqwest_error(error),
                    HttpRetryPolicy::ConnectOnly => error.is_connect(),
                },
                None,
            ),
        };
        let delay = hint.unwrap_or_else(|| retry_backoff(attempt));
        if !retryable
            || attempt == MAX_HTTP_ATTEMPTS
            || next.is_none()
            || delay > Duration::from_secs(60)
        {
            return result.map_err(|error| {
                sanitize_provider_error(
                    format!(
                        "{context}: Provider request failed after {attempt} attempt(s): {error}"
                    ),
                    secrets,
                )
            });
        }
        tracing::warn!(
            category = "agent",
            component = "agent.http",
            event_code = "PROVIDER_HTTP_RETRY",
            context,
            attempt,
            status = result
                .as_ref()
                .ok()
                .map(|response| response.status().as_u16()),
            delay_ms = delay.as_millis() as u64,
            outcome = "retrying",
            "Retrying provider HTTP request"
        );
        drop(result);
        tokio::time::sleep(delay).await;
        request = next.expect("clone checked above");
    }
    unreachable!("bounded attempts return")
}

pub fn sanitize_provider_error(message: impl Into<String>, secrets: &[&str]) -> String {
    let mut message = message.into();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        message = message.replace(secret, "[redacted]");
    }
    if message.chars().count() > MAX_PROVIDER_ERROR_CHARS {
        message = message
            .chars()
            .take(MAX_PROVIDER_ERROR_CHARS)
            .collect::<String>();
        message.push_str("...");
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[test]
    fn retry_after_and_hosted_request_policy() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "1.5".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_millis(1500)));
        headers.insert("retry-after-ms", "25".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_millis(25)));
        headers.remove("retry-after-ms");
        headers.insert("retry-after", "1e309".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(61)));
        headers.insert(
            "retry-after",
            httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_secs(30))
                .parse()
                .unwrap(),
        );
        assert!((28..=30).contains(&retry_after(&headers).unwrap().as_secs()));
        assert_eq!(
            generation_retry_policy(&serde_json::json!({"tools":[{"type":"web_search"}]})),
            HttpRetryPolicy::ConnectOnly
        );
        assert_eq!(
            generation_retry_policy(&serde_json::json!({"tools":[{"type":"function"}]})),
            HttpRetryPolicy::Replayable
        );
    }

    #[tokio::test]
    async fn http_status_retries_obey_replay_policy_and_server_delay() {
        use std::io::{Read, Write};
        for (statuses, policy, hint) in [
            (vec![429, 200], HttpRetryPolicy::Replayable, "0"),
            (vec![503, 503, 503], HttpRetryPolicy::Replayable, "0"),
            (vec![503], HttpRetryPolicy::ConnectOnly, "0"),
            (vec![429], HttpRetryPolicy::Replayable, "61"),
            (vec![400], HttpRetryPolicy::Replayable, "0"),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let expected = *statuses.last().unwrap();
            let server = std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                for status in statuses {
                    let mut socket = loop {
                        if let Ok((socket, _)) = listener.accept() {
                            break socket;
                        }
                        assert!(std::time::Instant::now() < deadline, "missing retry");
                        std::thread::sleep(Duration::from_millis(1));
                    };
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut buf = [0u8; 4096];
                    assert!(socket.read(&mut buf).unwrap() > 0);
                    write!(socket, "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\nRetry-After: {hint}\r\n\r\n").unwrap();
                }
            });
            let response = send_http_request("test", reqwest::Client::new().get(url), &[], policy)
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), expected);
            server.join().unwrap();
        }
    }

    struct FakeTransport {
        cloneable: bool,
        outcomes: Mutex<VecDeque<Result<Value, TransportFailure>>>,
        attempts: Mutex<Vec<u8>>,
    }

    #[async_trait::async_trait]
    impl HttpTransport for FakeTransport {
        async fn send(&self, attempt: u8) -> Result<Value, TransportFailure> {
            self.attempts.lock().unwrap().push(attempt);
            self.outcomes.lock().unwrap().pop_front().unwrap()
        }

        fn request_is_cloneable(&self) -> bool {
            self.cloneable
        }
    }

    fn failure(kind: TransportFailureKind) -> Result<Value, TransportFailure> {
        Err(TransportFailure::transient(kind, "transport failed"))
    }

    #[tokio::test]
    async fn transient_transport_retries_at_most_three_attempts() {
        let transport = FakeTransport {
            cloneable: true,
            outcomes: Mutex::new(VecDeque::from([
                failure(TransportFailureKind::Timeout),
                failure(TransportFailureKind::Connect),
                Ok(serde_json::json!({ "ok": true })),
            ])),
            attempts: Mutex::new(Vec::new()),
        };
        assert_eq!(send_with_retry(&transport).await.unwrap()["ok"], true);
        assert_eq!(*transport.attempts.lock().unwrap(), vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn non_transient_and_uncloneable_requests_do_not_retry() {
        for (cloneable, kind) in [
            (true, TransportFailureKind::NonTransient),
            (false, TransportFailureKind::Timeout),
        ] {
            let transport = FakeTransport {
                cloneable,
                outcomes: Mutex::new(VecDeque::from([failure(kind)])),
                attempts: Mutex::new(Vec::new()),
            };
            assert!(send_with_retry(&transport).await.is_err());
            assert_eq!(*transport.attempts.lock().unwrap(), vec![1]);
        }
    }

    #[test]
    fn tuning_redaction_bounds_and_backoff_match_preserved_contract() {
        let tuning = HttpClientTuning::default();
        assert!(tuning.http1_only);
        assert_eq!(tuning.connect_timeout, Duration::from_secs(30));
        assert_eq!(tuning.read_timeout, Duration::from_secs(180));
        assert_eq!(tuning.pool_idle_timeout, Duration::from_secs(90));
        assert!(build_http_client(tuning).is_ok());
        assert!((300..=400).contains(&retry_backoff(1).as_millis()));
        assert!((600..=700).contains(&retry_backoff(2).as_millis()));

        let secret = "secret-token";
        let sanitized =
            sanitize_provider_error(format!("{secret}:{}", "界".repeat(2_000)), &[secret]);
        assert!(!sanitized.contains(secret));
        assert!(sanitized.ends_with("..."));
        assert!(sanitized.is_char_boundary(sanitized.len()));
    }
}
