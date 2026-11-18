//! T-F012-001/004/005/007: loopback-only HTTP qualification, never live.
use std::net::TcpListener;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

use crate::anthropic::AnthropicOutcome;
use crate::openai::OpenAiOutcome;
use crate::qualification_http::{
    self, Dialect, FixtureAdmission, FixtureEndpoint, FixturePermit, Outcome,
};
use serde_json::json;

struct Grant;
impl FixtureAdmission for Grant {
    fn admit(&self, endpoint: &FixtureEndpoint) -> Result<FixturePermit, ()> {
        Ok(FixturePermit::for_fixture(endpoint))
    }
}
struct Deny;
impl FixtureAdmission for Deny {
    fn admit(&self, _: &FixtureEndpoint) -> Result<FixturePermit, ()> {
        Err(())
    }
}

fn endpoint(listener: &TcpListener) -> Result<String, Box<dyn std::error::Error>> {
    Ok(format!(
        "http://127.0.0.1:{}/fixture",
        listener.local_addr()?.port()
    ))
}

fn listener() -> Result<TcpListener, Box<dyn std::error::Error>> {
    Ok(TcpListener::bind("127.0.0.1:0")?)
}

async fn request(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut buf = [0_u8; 1024];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut buf)).await??;
        if n == 0 {
            break;
        }
        data.extend_from_slice(buf.get(..n).ok_or(std::io::ErrorKind::InvalidData)?);
        if let Some(header_end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(
                data.get(..header_end)
                    .ok_or(std::io::ErrorKind::InvalidData)?,
            );
            let length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if data.len() >= header_end + 4 + length {
                break;
            }
        }
        if data.len() > 8192 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    }
    Ok(data)
}

fn respond(
    listener: TcpListener,
    status: &str,
    headers: &str,
    body: &str,
    declared: usize,
) -> Result<JoinHandle<std::io::Result<Vec<u8>>>, std::io::Error> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {declared}\r\nConnection: close\r\n{headers}\r\n{body}"
    );
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    Ok(tokio::spawn(async move {
        let (mut stream, _) =
            tokio::time::timeout(Duration::from_secs(4), listener.accept()).await??;
        let request = request(&mut stream).await?;
        stream.write_all(response.as_bytes()).await?;
        Ok(request)
    }))
}

fn no_connection(listener: &TcpListener) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
        Ok(_) => Err(std::io::Error::other("unexpected connection")),
        Err(e) => Err(e),
    }
}

#[tokio::test]
async fn full_openai_and_anthropic_streams_reach_eof() -> Result<(), Box<dyn std::error::Error>> {
    for (dialect, fixture) in [
        (
            Dialect::OpenAi,
            include_str!("../fixtures/openai-responses-2026-11-18-function.sse"),
        ),
        (
            Dialect::Anthropic,
            include_str!("../fixtures/anthropic-messages-2026-11-18-tool.sse"),
        ),
    ] {
        let server = listener()?;
        let target = endpoint(&server)?;
        let worker = respond(
            server,
            "200 OK",
            "Content-Type: text/event-stream\r\n",
            fixture,
            fixture.len(),
        )?;
        let out = qualification_http::exchange(
            &target,
            dialect,
            &Grant,
            br#"{"fixture":true}"#,
            [("weather".to_owned(), json!({"type":"object"}))]
                .into_iter()
                .collect(),
        )
        .await;
        assert!(
            matches!(out, Outcome::OpenAi(OpenAiOutcome::Success { ref tools, .. }) | Outcome::Anthropic(AnthropicOutcome::Success { ref tools, .. }) if tools.len() == 1)
        );
        let seen = worker.await??;
        assert!(
            seen.windows(b"\"fixture\":true".len())
                .any(|s| s == b"\"fixture\":true")
        );
    }
    Ok(())
}

#[tokio::test]
async fn denied_or_bad_endpoint_never_connects() -> Result<(), Box<dyn std::error::Error>> {
    let server = listener()?;
    let target = endpoint(&server)?;
    assert!(matches!(
        qualification_http::exchange(
            &target,
            Dialect::OpenAi,
            &Deny,
            b"{}",
            std::collections::BTreeMap::new()
        )
        .await,
        Outcome::Denied
    ));
    no_connection(&server)?;
    for wrong in [
        target.replace("127.0.0.1", "localhost"),
        target.replace("/fixture", "/fixture?x=1"),
        target.replace("http://", "http://user@"),
        target.replace("/fixture", "/other"),
    ] {
        assert!(matches!(
            qualification_http::exchange(
                &wrong,
                Dialect::OpenAi,
                &Grant,
                b"{}",
                std::collections::BTreeMap::new()
            )
            .await,
            Outcome::InvalidEndpoint
        ));
    }
    no_connection(&server)?;
    Ok(())
}

#[tokio::test]
async fn rate_limit_is_observed_without_retry() -> Result<(), Box<dyn std::error::Error>> {
    let server = listener()?;
    let watch = server.try_clone()?;
    let target = endpoint(&server)?;
    let worker = respond(
        server,
        "429 Too Many Requests",
        "Retry-After: 17\r\n",
        "",
        0,
    )?;
    assert!(matches!(
        qualification_http::exchange(
            &target,
            Dialect::OpenAi,
            &Grant,
            b"{}",
            std::collections::BTreeMap::new()
        )
        .await,
        Outcome::RateLimited(Some(17))
    ));
    worker.await??;
    no_connection(&watch)?;
    Ok(())
}

#[tokio::test]
async fn redirect_never_sends_request_or_credentials_to_target()
-> Result<(), Box<dyn std::error::Error>> {
    let server = listener()?;
    let destination = listener()?;
    let target = endpoint(&server)?;
    let location = endpoint(&destination)?;
    let headers = format!("Location: {location}\r\n");
    let worker = respond(server, "307 Temporary Redirect", &headers, "", 0)?;
    assert!(matches!(
        qualification_http::exchange(
            &target,
            Dialect::OpenAi,
            &Grant,
            b"fixture-body",
            std::collections::BTreeMap::new()
        )
        .await,
        Outcome::RedirectRefused
    ));
    let seen = worker.await??;
    assert!(
        String::from_utf8_lossy(&seen)
            .to_lowercase()
            .contains("authorization: bearer fixture-only")
    );
    no_connection(&destination)?;
    Ok(())
}

#[tokio::test]
async fn dropped_or_truncated_body_is_unknown_without_proposal()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = include_str!("../fixtures/openai-responses-2026-11-18-function.sse");
    let server = listener()?;
    let watch = server.try_clone()?;
    let target = endpoint(&server)?;
    let worker = respond(
        server,
        "200 OK",
        "Content-Type: text/event-stream\r\n",
        fixture,
        fixture.len() + 10,
    )?;
    assert!(matches!(
        qualification_http::exchange(
            &target,
            Dialect::OpenAi,
            &Grant,
            b"{}",
            [("weather".to_owned(), json!({"type":"object"}))]
                .into_iter()
                .collect()
        )
        .await,
        Outcome::Unknown
    ));
    worker.await??;
    no_connection(&watch)?;
    Ok(())
}

#[tokio::test]
async fn finite_failures_and_body_bounds_are_classified() -> Result<(), Box<dyn std::error::Error>>
{
    let server = listener()?;
    let target = endpoint(&server)?;
    let worker = respond(server, "503 Service Unavailable", "", "", 0)?;
    assert!(matches!(
        qualification_http::exchange(
            &target,
            Dialect::OpenAi,
            &Grant,
            b"{}",
            std::collections::BTreeMap::new()
        )
        .await,
        Outcome::HttpStatus(503)
    ));
    worker.await??;

    let server = listener()?;
    let target = endpoint(&server)?;
    assert!(matches!(
        qualification_http::exchange(
            &target,
            Dialect::OpenAi,
            &Grant,
            &vec![0; 4097],
            std::collections::BTreeMap::new()
        )
        .await,
        Outcome::Protocol
    ));
    no_connection(&server)?;

    let server = listener()?;
    let target = endpoint(&server)?;
    let worker = respond(
        server,
        "200 OK",
        "Content-Type: text/event-stream\r\n",
        "",
        1_048_577,
    )?;
    assert!(matches!(
        qualification_http::exchange(
            &target,
            Dialect::OpenAi,
            &Grant,
            b"{}",
            std::collections::BTreeMap::new()
        )
        .await,
        Outcome::Protocol
    ));
    worker.await??;

    let server = listener()?;
    let target = endpoint(&server)?;
    let body = "data: unfinished\n\n";
    let worker = respond(
        server,
        "200 OK",
        "Content-Type: text/event-stream\r\n",
        body,
        body.len(),
    )?;
    assert!(matches!(
        qualification_http::exchange(
            &target,
            Dialect::OpenAi,
            &Grant,
            b"{}",
            std::collections::BTreeMap::new()
        )
        .await,
        Outcome::Protocol
    ));
    worker.await??;
    Ok(())
}
