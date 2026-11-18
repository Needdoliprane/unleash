//! T-F012-004/005/011: bounded subset of Responses streaming fixtures.
use crate::StreamError;
use crate::openai::OpenAiUsage;
use crate::openai::{OpenAiOutcome, OpenAiStream};
use serde_json::json;

fn run(fixture: &str, known_tool: bool) -> Result<Option<OpenAiOutcome>, StreamError> {
    let contracts = if known_tool {
        [("weather".to_owned(), json!({"type":"object","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false}))].into_iter().collect()
    } else {
        std::collections::BTreeMap::default()
    };
    let mut stream = OpenAiStream::new(contracts)?;
    for byte in fixture.as_bytes() {
        stream.push(std::slice::from_ref(byte))?;
    }
    let result = stream.finish()?;
    assert_eq!(stream.finish()?, None);
    Ok(result)
}

#[test]
fn sequence_and_item_id_mismatches_are_rejected() {
    let fixture = include_str!("../fixtures/openai-responses-2026-11-18-function.sse");
    let skipped = fixture.replacen("\"sequence_number\":4", "\"sequence_number\":5", 1);
    assert_eq!(run(&skipped, true), Err(StreamError::InvalidEvent));
    let wrong_id = fixture.replacen("\"item_id\":\"fc_1\"", "\"item_id\":\"fc_other\"", 1);
    assert_eq!(run(&wrong_id, true), Err(StreamError::InvalidEvent));
}

#[test]
fn terminal_and_partial_following_event_in_one_chunk_never_emit()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = include_str!("../fixtures/openai-responses-2026-11-18-function.sse");
    let contracts = [("weather".to_owned(), json!({"type":"object"}))]
        .into_iter()
        .collect();
    let mut stream = OpenAiStream::new(contracts)?;
    let malformed = format!("{fixture}event: response.incomplete\ndata: {{");
    assert_eq!(
        stream.push(malformed.as_bytes()),
        Err(StreamError::Interrupted)
    );
    assert_eq!(stream.finish(), Err(StreamError::InvalidEvent));
    let mut stream = OpenAiStream::new(
        [("weather".to_owned(), json!({"type":"object"}))]
            .into_iter()
            .collect(),
    )?;
    stream.push(fixture.as_bytes())?;
    assert_eq!(
        stream.push(b"event: response.incomplete\ndata: {"),
        Err(StreamError::InvalidEvent)
    );
    assert_eq!(stream.finish(), Err(StreamError::InvalidEvent));
    Ok(())
}

#[test]
fn text_fixture_and_detailed_usage() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = include_str!("../fixtures/openai-responses-2026-11-18-text.sse");
    assert!(
        matches!(run(fixture, true)?, Some(OpenAiOutcome::Success { text, tools, usage: Some(OpenAiUsage { input: 0, output: 2, total: 2, cache: Some(0), cache_write: Some(0), reasoning: Some(1) }) }) if text == "Héllo" && tools.is_empty())
    );
    let partial = fixture.replace("\"input_tokens_details\":{\"cached_tokens\":0,\"cache_write_tokens\":0},\"output_tokens_details\":{\"reasoning_tokens\":1}", "\"input_tokens_details\":null,\"output_tokens_details\":null");
    assert!(matches!(
        run(&partial, true)?,
        Some(OpenAiOutcome::Success {
            usage: Some(OpenAiUsage {
                input: 0,
                output: 2,
                total: 2,
                cache: None,
                cache_write: None,
                reasoning: None
            }),
            ..
        })
    ));
    let unknown = fixture.replace("\"cached_tokens\":0", "\"cached_tokens\":0,\"extra\":1");
    assert_eq!(run(&unknown, true), Err(StreamError::InvalidEvent));
    Ok(())
}

#[test]
fn function_arguments_require_done_item_and_completion() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = include_str!("../fixtures/openai-responses-2026-11-18-function.sse");
    assert!(matches!(
        run(&fixture.replace('\n', "\r\n"), true)?,
        Some(OpenAiOutcome::Success { .. })
    ));
    assert!(
        matches!(run(fixture, true)?, Some(OpenAiOutcome::Success { tools, usage: None, .. }) if tools.len() == 1 && tools.first().is_some_and(|p| p.id == "call_1" && p.name == "weather" && p.arguments == json!({"city":"Paris"})))
    );
    assert_eq!(
        run(fixture, false)?,
        Some(OpenAiOutcome::InvalidSchema { usage: None })
    );
    let mismatch = fixture.replacen("\\\"Paris\\\"}", "\\\"Lyon\\\"}", 1);
    assert!(run(&mismatch, true).is_err());
    let cut = fixture
        .split("event: response.completed")
        .next()
        .ok_or(StreamError::InvalidEvent)?;
    assert_eq!(run(cut, true), Err(StreamError::Interrupted));
    Ok(())
}

#[test]
fn refused_incomplete_unknown_and_mismatched_event_fail_closed()
-> Result<(), Box<dyn std::error::Error>> {
    assert!(
        matches!(run(include_str!("../fixtures/openai-responses-2026-11-18-refusal.sse"), true)?, Some(OpenAiOutcome::Refusal { reason, usage: None }) if reason == "Non")
    );
    assert_eq!(
        run(
            include_str!("../fixtures/openai-responses-2026-11-18-incomplete.sse"),
            true
        )?,
        Some(OpenAiOutcome::Truncated { usage: None })
    );
    let fixture = include_str!("../fixtures/openai-responses-2026-11-18-text.sse");
    let unknown = fixture.replacen(
        "\"sequence_number\":0",
        "\"unrecognized\":true,\"sequence_number\":0",
        1,
    );
    assert_eq!(run(&unknown, true), Err(StreamError::InvalidEvent));
    let wrong = fixture.replacen("event: response.created", "event: response.completed", 1);
    assert_eq!(run(&wrong, true), Err(StreamError::InvalidEvent));
    let contradictory = fixture.replacen(
        "\"incomplete_details\":null",
        "\"incomplete_details\":{\"reason\":\"max_output_tokens\"}",
        1,
    );
    assert_eq!(run(&contradictory, true), Err(StreamError::InvalidEvent));
    Ok(())
}
