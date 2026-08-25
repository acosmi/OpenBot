//! Provider adapters 的共享 bounded request 与 immediate terminal session。

use async_trait::async_trait;
use core::time::Duration;
use http::HeaderValue;
use http::header::RETRY_AFTER;
use openbot_application::{
    ProviderEvent, ProviderMessageRole, ProviderPortError, ProviderRequest, ProviderSession,
};
use std::time::SystemTime;

use crate::net::safe_http::{SafeHttpError, SafeHttpStreamResponse};

pub(crate) const MAX_PROVIDER_MESSAGES: usize = 4096;
pub(crate) const MAX_PROVIDER_TOOLS: usize = 256;
pub(crate) const MAX_PROVIDER_FIELD_BYTES: usize = 1024 * 1024;

pub(crate) fn validate_request(request: &ProviderRequest) -> Result<(), ProviderPortError> {
    if request.messages.is_empty()
        || request.messages.len() > MAX_PROVIDER_MESSAGES
        || request.tools.len() > MAX_PROVIDER_TOOLS
        || request.max_output_tokens == Some(0)
    {
        return Err(ProviderPortError::InvalidRequest {
            field: "provider_request",
        });
    }
    for message in &request.messages {
        if message.content.len() > MAX_PROVIDER_FIELD_BYTES
            || message.content.as_bytes().contains(&0)
            || (message.role == ProviderMessageRole::Tool) != message.tool_call_id.is_some()
            || (message.role == ProviderMessageRole::Tool) != message.tool_name.is_some()
        {
            return Err(ProviderPortError::InvalidRequest {
                field: "provider_message",
            });
        }
    }
    for tool in &request.tools {
        if tool.name.is_empty()
            || tool.name.len() > 256
            || tool.name.as_bytes().contains(&0)
            || tool.description.len() > 64 * 1024
            || !tool.input_schema.is_object()
        {
            return Err(ProviderPortError::InvalidRequest {
                field: "provider_tool",
            });
        }
    }
    Ok(())
}

pub(crate) struct ImmediateSession {
    event: Option<ProviderEvent>,
}

impl ImmediateSession {
    pub(crate) const fn new(event: ProviderEvent) -> Self {
        Self { event: Some(event) }
    }
}

#[async_trait]
impl ProviderSession for ImmediateSession {
    async fn next_event(&mut self) -> Result<Option<ProviderEvent>, ProviderPortError> {
        Ok(self.event.take())
    }
}

pub(crate) fn map_start_error(error: SafeHttpError) -> ProviderPortError {
    match error {
        SafeHttpError::DnsUnavailable | SafeHttpError::ConnectFailed | SafeHttpError::TlsFailed => {
            ProviderPortError::Unavailable
        }
        SafeHttpError::DeadlineExceeded
        | SafeHttpError::ProtocolFailed
        | SafeHttpError::ResponseTooLarge
        | SafeHttpError::StreamStalled => ProviderPortError::CommitUnknown,
        SafeHttpError::InvalidUrl
        | SafeHttpError::SchemeRejected
        | SafeHttpError::InvalidBudget
        | SafeHttpError::InvalidHeader
        | SafeHttpError::InvalidAllowlist
        | SafeHttpError::DestinationDenied
        | SafeHttpError::PeerMismatch
        | SafeHttpError::RedirectInvalid
        | SafeHttpError::RedirectLimit
        | SafeHttpError::RedirectMethodRejected
        | SafeHttpError::SensitiveRedirectRejected => ProviderPortError::InvalidRequest {
            field: "provider_transport",
        },
    }
}

pub(crate) fn response_retry_after(response: &SafeHttpStreamResponse) -> Option<Duration> {
    response
        .header(&RETRY_AFTER)
        .and_then(|value| parse_retry_after(value, SystemTime::now()))
}

fn parse_retry_after(value: &HeaderValue, now: SystemTime) -> Option<Duration> {
    let value = value.to_str().ok()?.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = httpdate::parse_http_date(value).ok()?;
    Some(at.duration_since(now).unwrap_or(Duration::ZERO))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_accepts_delta_or_http_date_and_rejects_vendor_prose() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert_eq!(
            parse_retry_after(&HeaderValue::from_static("12"), now),
            Some(Duration::from_secs(12))
        );
        let future = now + Duration::from_secs(30);
        let date = HeaderValue::from_str(&httpdate::fmt_http_date(future)).unwrap();
        assert_eq!(parse_retry_after(&date, now), Some(Duration::from_secs(30)));
        assert_eq!(
            parse_retry_after(&HeaderValue::from_static("try again soon"), now),
            None
        );
    }

    #[test]
    fn pre_send_and_commit_unknown_transport_failures_are_distinct() {
        assert_eq!(
            map_start_error(SafeHttpError::ConnectFailed),
            ProviderPortError::Unavailable
        );
        assert_eq!(
            map_start_error(SafeHttpError::DeadlineExceeded),
            ProviderPortError::CommitUnknown
        );
        assert!(matches!(
            map_start_error(SafeHttpError::DestinationDenied),
            ProviderPortError::InvalidRequest { .. }
        ));
    }
}
