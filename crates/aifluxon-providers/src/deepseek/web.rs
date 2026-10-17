//! DeepSeek's standalone Search/Fetch APIs; independent of model API mode.

use crate::common::http::{
    build_http_client, sanitize_provider_error, send_http_request, HttpClientTuning,
    HttpRetryPolicy,
};
use aifluxon_core::ProviderError;
use serde_json::Value;
use std::time::Duration;

const MAX_WEB_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct DeepSeekWebClient {
    client: reqwest::Client,
    tools_url: String,
    api_key: String,
}

impl DeepSeekWebClient {
    pub fn new(base_url: &str, api_key: &str) -> Result<Self, ProviderError> {
        let base = base_url.trim().trim_end_matches('/');
        let url = reqwest::Url::parse(base).map_err(|_| {
            ProviderError::message("DeepSeek Search/Fetch requires a valid Base URL.")
        })?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ProviderError::message("DeepSeek Search/Fetch requires an HTTP(S) Base URL without credentials, query or fragment."));
        }
        Ok(Self {
            client: build_http_client(HttpClientTuning::default())
                .map_err(ProviderError::message)?,
            tools_url: format!("{}/v1/tools", base.strip_suffix("/v1").unwrap_or(base)),
            api_key: api_key.to_string(),
        })
    }

    /// Arguments are forwarded unchanged so the service validates its contract.
    /// This client never fetches the target URL itself or forwards credentials to it.
    pub async fn execute(&self, tool: &str, arguments: Value) -> Result<Value, ProviderError> {
        let operation = match tool {
            "web_search" => "search",
            "web_fetch" => "fetch",
            _ => return Err(ProviderError::message("Unknown DeepSeek web tool.")),
        };
        let context = format!("DeepSeek {operation}");
        let response = send_http_request(
            &context,
            self.client
                .post(format!("{}/{operation}", self.tools_url))
                .bearer_auth(&self.api_key)
                .timeout(Duration::from_secs(180))
                .json(&arguments),
            &[&self.api_key],
            HttpRetryPolicy::Replayable,
        )
        .await
        .map_err(ProviderError::message)?;
        let status = response.status();
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            ProviderError::message(sanitize_provider_error(
                format!("{context}: {error}"),
                &[&self.api_key],
            ))
        })? {
            if bytes.len().saturating_add(chunk.len()) > MAX_WEB_RESPONSE_BYTES {
                return Err(ProviderError::message(format!(
                    "{context}: response exceeds 16 MiB."
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(ProviderError::message(sanitize_provider_error(
                format!(
                    "{context}: HTTP {}: {}",
                    status.as_u16(),
                    String::from_utf8_lossy(&bytes)
                ),
                &[&self.api_key],
            )));
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ProviderError::message(format!("{context}: invalid JSON response.")))?;
        let valid = match operation {
            "search" => value.get("results").is_some_and(Value::is_array),
            _ => {
                value.get("title").is_some_and(Value::is_string)
                    && value.get("content").is_some_and(Value::is_string)
            }
        };
        if !valid {
            return Err(ProviderError::message(format!(
                "{context}: invalid response schema."
            )));
        }
        Ok(value)
    }
}
