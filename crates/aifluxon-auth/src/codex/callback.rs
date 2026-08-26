use crate::error::{AuthError, AuthErrorKind};
use std::collections::HashMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Duration, Instant};
use url::Url;

pub const CALLBACK_PORTS: [u16; 2] = [1455, 1457];
pub const CALLBACK_PATH: &str = "/auth/callback";
pub const LOGIN_TIMEOUT_SECS: u64 = 300;

pub async fn bind_callback_listener() -> Result<(TcpListener, u16), AuthError> {
    let mut errors = Vec::new();
    for port in CALLBACK_PORTS {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return Ok((listener, port)),
            Err(error) => errors.push(format!("{port}: {error}")),
        }
    }
    Err(AuthError::new(
        AuthErrorKind::CallbackBind,
        format!(
            "Could not bind the Codex login callback on 127.0.0.1:1455 or 1457: {}",
            errors.join("; ")
        ),
    ))
}

pub fn callback_code_from_target(target: &str, expected_state: &str) -> Result<String, AuthError> {
    let url = Url::parse(&format!("http://localhost{target}")).map_err(|_| {
        AuthError::new(
            AuthErrorKind::CallbackProtocol,
            "Codex login callback address is invalid.",
        )
    })?;
    if url.path() != CALLBACK_PATH {
        return Err(AuthError::new(
            AuthErrorKind::CallbackProtocol,
            "Codex login callback path is invalid.",
        ));
    }
    let params = url.query_pairs().collect::<HashMap<_, _>>();
    if params.get("state").map(|value| value.as_ref()) != Some(expected_state) {
        return Err(AuthError::new(
            AuthErrorKind::StateMismatch,
            "Codex login callback state did not match.",
        ));
    }
    if let Some(error) = params.get("error") {
        let description = params
            .get("error_description")
            .map(|value| value.as_ref())
            .unwrap_or(error.as_ref());
        return Err(AuthError::new(
            AuthErrorKind::CallbackProtocol,
            format!("Codex authorization failed: {description}"),
        ));
    }
    params
        .get("code")
        .map(|value| value.to_string())
        .filter(|code| !code.trim().is_empty())
        .ok_or_else(|| {
            AuthError::new(
                AuthErrorKind::CallbackProtocol,
                "Codex login callback is missing the authorization code.",
            )
        })
}

pub async fn write_callback_response(stream: &mut TcpStream, ok: bool) {
    let (title, message) = if ok {
        (
            "Codex authorization complete",
            "You can close this page and return to the application.",
        )
    } else {
        (
            "Codex authorization failed",
            "Return to the application to see the error and try again.",
        )
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head><body><h1>{title}</h1><p>{message}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

pub async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<(String, TcpStream), AuthError> {
    let deadline = Instant::now() + timeout;
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(AuthError::new(
            AuthErrorKind::CallbackTimeout,
            "Codex authorization timed out.",
        ));
    }
    let (mut stream, _) = tokio::time::timeout(remaining, listener.accept())
        .await
        .map_err(|_| {
            AuthError::new(
                AuthErrorKind::CallbackTimeout,
                "Codex authorization timed out.",
            )
        })?
        .map_err(|error| {
            AuthError::new(
                AuthErrorKind::CallbackProtocol,
                format!("Codex login callback accept failed: {error}"),
            )
        })?;
    let mut request = Vec::new();
    let mut chunk = [0_u8; 2048];
    while request.len() < 16 * 1024 && !request.windows(4).any(|part| part == b"\r\n\r\n") {
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .map_err(|_| {
                AuthError::new(
                    AuthErrorKind::CallbackTimeout,
                    "Codex login callback read timed out.",
                )
            })?
            .map_err(|error| {
                AuthError::new(
                    AuthErrorKind::CallbackProtocol,
                    format!("Codex login callback read failed: {error}"),
                )
            })?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
    }
    let first_line = String::from_utf8_lossy(&request)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let target = first_line.split_whitespace().nth(1).unwrap_or_default();
    match callback_code_from_target(target, expected_state) {
        Ok(code) => Ok((code, stream)),
        Err(error) => {
            write_callback_response(&mut stream, false).await;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_only_accepts_expected_path() {
        let error = callback_code_from_target("/other?state=abc&code=1", "abc").unwrap_err();
        assert_eq!(error.kind(), AuthErrorKind::CallbackProtocol);
    }

    #[test]
    fn oauth_state_mismatch_is_rejected() {
        let error = callback_code_from_target("/auth/callback?state=wrong&code=abc", "expected")
            .unwrap_err();
        assert_eq!(error.kind(), AuthErrorKind::StateMismatch);
    }

    #[test]
    fn callback_error_parameter_propagates() {
        let error = callback_code_from_target(
            "/auth/callback?state=abc&error=access_denied&error_description=nope",
            "abc",
        )
        .unwrap_err();
        assert!(error.to_string().contains("nope"));
    }
}
