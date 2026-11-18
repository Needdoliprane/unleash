//! Test-only HTTP harness. This is not a model-router authority, ledger,
//! budget reservation, production transport, TLS implementation, or API.
//! Endpoints are constrained to literal IPv4 loopback fixture paths.

use crate::{
    StreamError,
    anthropic::{AnthropicOutcome, AnthropicStream},
    openai::{OpenAiOutcome, OpenAiStream},
};
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};

const MAX_REQUEST: usize = 4096;
const MAX_RESPONSE: usize = 1_048_576;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Dialect {
    OpenAi,
    Anthropic,
}

#[derive(Debug)]
pub(crate) enum Outcome {
    OpenAi(OpenAiOutcome),
    Anthropic(AnthropicOutcome),
    Denied,
    InvalidEndpoint,
    RateLimited(Option<u64>),
    RedirectRefused,
    HttpStatus(u16),
    Protocol,
    Unknown,
}

#[derive(Debug)]
pub(crate) struct FixtureEndpoint(reqwest::Url);

impl FixtureEndpoint {
    fn parse(input: &str) -> Option<Self> {
        let url = reqwest::Url::parse(input).ok()?;
        if url.as_str() != input
            || url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/fixture"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return None;
        }
        Some(Self(url))
    }
}

/// A synthetic one-use admission artifact, scoped solely to this test bench.
pub(crate) struct FixturePermit(String);
impl FixturePermit {
    pub(crate) fn for_fixture(endpoint: &FixtureEndpoint) -> Self {
        Self(endpoint.0.as_str().to_owned())
    }
}
pub(crate) trait FixtureAdmission {
    fn admit(&self, endpoint: &FixtureEndpoint) -> Result<FixturePermit, ()>;
}

enum Parser {
    OpenAi(OpenAiStream),
    Anthropic(AnthropicStream),
}
impl Parser {
    fn push(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        match self {
            Self::OpenAi(s) => s.push(bytes),
            Self::Anthropic(s) => s.push(bytes),
        }
    }
    fn finish(&mut self) -> Result<Outcome, StreamError> {
        match self {
            Self::OpenAi(s) => s
                .finish()?
                .map(Outcome::OpenAi)
                .ok_or(StreamError::Interrupted),
            Self::Anthropic(s) => s
                .finish()?
                .map(Outcome::Anthropic)
                .ok_or(StreamError::Interrupted),
        }
    }
}

pub(crate) async fn exchange(
    input: &str,
    dialect: Dialect,
    gate: &dyn FixtureAdmission,
    body: &[u8],
    contracts: BTreeMap<String, Value>,
) -> Outcome {
    let Some(endpoint) = FixtureEndpoint::parse(input) else {
        return Outcome::InvalidEndpoint;
    };
    if body.len() > MAX_REQUEST {
        return Outcome::Protocol;
    }
    let Ok(permit) = gate.admit(&endpoint) else {
        return Outcome::Denied;
    };
    if permit.0 != endpoint.0.as_str() {
        return Outcome::Denied;
    }
    let mut parser = match dialect {
        Dialect::OpenAi => match OpenAiStream::new(contracts) {
            Ok(v) => Parser::OpenAi(v),
            Err(_) => return Outcome::Protocol,
        },
        Dialect::Anthropic => match AnthropicStream::new(contracts) {
            Ok(v) => Parser::Anthropic(v),
            Err(_) => return Outcome::Protocol,
        },
    };
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(2))
        .read_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(4))
        .build()
    else {
        return Outcome::Protocol;
    };
    // A fixture-only constant, never a credential. No caller controls headers.
    let response = client
        .post(endpoint.0)
        .header(reqwest::header::AUTHORIZATION, "Bearer fixture-only")
        .body(body.to_vec())
        .send()
        .await;
    let Ok(mut response) = response else {
        return Outcome::Unknown;
    };
    let status = response.status();
    if status.is_redirection() {
        return Outcome::RedirectRefused;
    }
    if status.as_u16() == 429 {
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|n| *n <= 300);
        return Outcome::RateLimited(retry_after);
    }
    if !status.is_success() {
        return Outcome::HttpStatus(status.as_u16());
    }
    let event_stream = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("text/event-stream"));
    if !event_stream {
        return Outcome::Protocol;
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE as u64)
    {
        return Outcome::Protocol;
    }
    let mut bytes = 0_usize;
    loop {
        let Ok(chunk) = response.chunk().await else {
            return Outcome::Unknown;
        };
        let Some(chunk) = chunk else {
            break;
        };
        bytes = bytes.saturating_add(chunk.len());
        if bytes > MAX_RESPONSE {
            return Outcome::Protocol;
        }
        if parser.push(&chunk).is_err() {
            return Outcome::Protocol;
        }
    }
    match parser.finish() {
        Ok(outcome) => outcome,
        Err(StreamError::Interrupted) => Outcome::Unknown,
        Err(_) => Outcome::Protocol,
    }
}
