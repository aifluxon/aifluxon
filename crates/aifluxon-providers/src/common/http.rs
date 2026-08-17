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
        .use_native_tls()
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
    Duration::from_millis(300 * u64::from(attempt.max(1)))
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
        assert_eq!(retry_backoff(1), Duration::from_millis(300));
        assert_eq!(retry_backoff(2), Duration::from_millis(600));

        let secret = "secret-token";
        let sanitized =
            sanitize_provider_error(format!("{secret}:{}", "界".repeat(2_000)), &[secret]);
        assert!(!sanitized.contains(secret));
        assert!(sanitized.ends_with("..."));
        assert!(sanitized.is_char_boundary(sanitized.len()));
    }
}
