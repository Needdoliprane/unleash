//! Partial T-F012-004/005/011: synthetic Anthropic Messages fixtures.
use crate::StreamError;
use crate::anthropic::{AnthropicOutcome, AnthropicStream, AnthropicUsage, StopReason};

// T-F012-011 / REQ-MODEL-011: a rejected event must not corrupt partial usage.
#[test]
fn t_f012_011_usage_snapshot_is_atomic_and_partial() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = include_str!("../fixtures/anthropic-messages-2026-11-18-text.sse")
        .replace("\"input_tokens\":0", "\"input_tokens\":10")
        .replace(",\"cache_read_input_tokens\":0", "");
    let partial = fixture
        .split("event: message_stop")
        .next()
        .ok_or(StreamError::InvalidEvent)?;
    let mut stream = AnthropicStream::new(std::collections::BTreeMap::new())?;
    assert_eq!(stream.observed_usage(), None);
    stream.push(partial.as_bytes())?;
    let expected = AnthropicUsage {
        input: Some(10),
        output: Some(3),
        cache_read: None,
        cache_creation: None,
    };
    assert_eq!(stream.observed_usage(), Some(expected.clone()));
    let invalid = b"event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"input_tokens\":20,\"output_tokens\":2}}\n\n";
    assert_eq!(stream.push(invalid), Err(StreamError::InvalidEvent));
    assert_eq!(stream.observed_usage(), Some(expected.clone()));
    assert_eq!(stream.finish(), Err(StreamError::InvalidEvent));
    let mut interrupted = AnthropicStream::new(std::collections::BTreeMap::new())?;
    interrupted.push(partial.as_bytes())?;
    assert_eq!(interrupted.finish(), Err(StreamError::Interrupted));
    assert_eq!(interrupted.observed_usage(), Some(expected));
    Ok(())
}
use serde_json::json;

#[test]
fn trailing_comment_rejected_at_every_chunk_split() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = format!(
        "{}: heartbeat\n\n",
        include_str!("../fixtures/anthropic-messages-2026-11-18-tool.sse")
    );
    for split in 0..=fixture.len() {
        if !fixture.is_char_boundary(split) {
            continue;
        }
        let mut stream = AnthropicStream::new(
            [("weather".to_owned(), json!({"type":"object"}))]
                .into_iter()
                .collect(),
        )?;
        let (left, right) = fixture.split_at(split);
        let first = stream.push(left.as_bytes());
        if first.is_ok() {
            assert_eq!(
                stream.push(right.as_bytes()),
                Err(StreamError::InvalidEvent),
                "split {split}"
            );
        } else {
            assert_eq!(first, Err(StreamError::InvalidEvent), "split {split}");
        }
        assert_eq!(stream.finish(), Err(StreamError::InvalidEvent));
    }
    Ok(())
}

fn run(data: &str, known: bool) -> Result<Option<AnthropicOutcome>, StreamError> {
    let contracts = if known {
        [("weather".to_owned(), json!({"type":"object","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false}))].into_iter().collect()
    } else {
        std::collections::BTreeMap::new()
    };
    let mut stream = AnthropicStream::new(contracts)?;
    for byte in data.as_bytes() {
        stream.push(std::slice::from_ref(byte))?;
    }
    let outcome = stream.finish()?;
    assert_eq!(stream.finish()?, None);
    Ok(outcome)
}

#[test]
fn text_stop_and_cumulative_usage_are_preserved() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = include_str!("../fixtures/anthropic-messages-2026-11-18-text.sse");
    assert!(
        matches!(run(fixture, true)?, Some(AnthropicOutcome::Success { text, tools, stop: StopReason::EndTurn, usage: AnthropicUsage { input: Some(0), output: Some(3), cache_read: Some(0), cache_creation: None } }) if text == "Héllo" && tools.is_empty())
    );
    assert!(matches!(
        run(&fixture.replace('\n', "\r\n"), true)?,
        Some(AnthropicOutcome::Success { .. })
    ));
    Ok(())
}

#[test]
fn tool_requires_contract_complete_json_and_eof() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = include_str!("../fixtures/anthropic-messages-2026-11-18-tool.sse");
    assert!(
        matches!(run(fixture, true)?, Some(AnthropicOutcome::Success { stop: StopReason::ToolUse, tools, .. }) if tools.len()==1 && tools.first().is_some_and(|t| t.id == "toolu_1" && t.arguments == json!({"city":"Paris"})))
    );
    assert!(matches!(
        run(fixture, false)?,
        Some(AnthropicOutcome::InvalidSchema { .. })
    ));
    let cut = fixture
        .split("event: message_stop")
        .next()
        .ok_or(StreamError::InvalidEvent)?;
    assert_eq!(run(cut, true), Err(StreamError::Interrupted));
    let invalid = fixture.replacen("\\\"Paris\\\"}", "123}", 1);
    assert!(matches!(
        run(&invalid, true)?,
        Some(AnthropicOutcome::InvalidSchema { .. })
    ));
    let contradiction = fixture.replacen(
        "\"stop_reason\":\"tool_use\"",
        "\"stop_reason\":\"end_turn\"",
        1,
    );
    assert_eq!(run(&contradiction, true), Err(StreamError::InvalidEvent));
    Ok(())
}

#[test]
fn refusal_truncation_unknowns_duplicates_and_partial_suffix_are_safe()
-> Result<(), Box<dyn std::error::Error>> {
    let text = include_str!("../fixtures/anthropic-messages-2026-11-18-text.sse");
    let refusal = text.replace(
        "\"stop_reason\":\"end_turn\"",
        "\"stop_reason\":\"refusal\"",
    );
    assert!(matches!(
        run(&refusal, true)?,
        Some(AnthropicOutcome::Refusal { .. })
    ));
    for reason in ["max_tokens", "model_context_window_exceeded"] {
        let f = text.replace(
            "\"stop_reason\":\"end_turn\"",
            &format!("\"stop_reason\":\"{reason}\""),
        );
        assert!(matches!(
            run(&f, true)?,
            Some(AnthropicOutcome::Truncated { .. })
        ));
    }
    let unknown = text.replacen("\"type\":\"ping\"", "\"type\":\"ping\",\"extra\":true", 1);
    assert_eq!(run(&unknown, true), Err(StreamError::InvalidEvent));
    let duplicate = format!("{text}event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n");
    assert_eq!(run(&duplicate, true), Err(StreamError::InvalidEvent));
    let suffix = format!("{text}event: message_delta\ndata: {{");
    let mut stream = AnthropicStream::new(std::collections::BTreeMap::new())?;
    assert!(stream.push(suffix.as_bytes()).is_err());
    assert_eq!(stream.finish(), Err(StreamError::InvalidEvent));
    Ok(())
}

#[test]
fn counters_stop_sequence_and_declared_refusal_are_distinct()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = include_str!("../fixtures/anthropic-messages-2026-11-18-text.sse");
    let absent = fixture
        .replace("\"input_tokens\":0,", "")
        .replace(",\"cache_read_input_tokens\":0", "");
    assert!(matches!(
        run(&absent, true)?,
        Some(AnthropicOutcome::Success {
            usage: AnthropicUsage {
                input: None,
                cache_read: None,
                ..
            },
            ..
        })
    ));
    let sequence = fixture.replace(
        "\"stop_reason\":\"end_turn\",\"stop_sequence\":null",
        "\"stop_reason\":\"stop_sequence\",\"stop_sequence\":\"END\"",
    );
    assert!(
        matches!(run(&sequence, true)?, Some(AnthropicOutcome::Success { stop: StopReason::StopSequence(sequence), .. }) if sequence == "END")
    );
    let refused = fixture.replace("\"stop_reason\":\"end_turn\",\"stop_sequence\":null", "\"stop_reason\":\"refusal\",\"stop_sequence\":null,\"stop_details\":{\"type\":\"refusal\",\"category\":\"cyber\",\"explanation\":\"declined\"}");
    assert!(
        matches!(run(&refused, true)?, Some(AnthropicOutcome::Refusal { explanation: Some(explanation), .. }) if explanation == "declined")
    );
    let smaller = fixture.replace("event: message_stop", "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":null,\"stop_sequence\":null},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop");
    assert_eq!(run(&smaller, true), Err(StreamError::InvalidEvent));
    Ok(())
}

#[test]
fn rejects_correlated_ids_nonobjects_and_post_terminal_data()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = include_str!("../fixtures/anthropic-messages-2026-11-18-tool.sse");
    let wrong_index = fixture.replacen("\"index\":0,\"delta\"", "\"index\":1,\"delta\"", 1);
    assert_eq!(run(&wrong_index, true), Err(StreamError::InvalidEvent));
    let duplicate_id = fixture.replace("event: message_delta", "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"weather\",\"input\":{}}}\n\nevent: message_delta");
    assert_eq!(run(&duplicate_id, true), Err(StreamError::InvalidEvent));
    let array = fixture
        .replace("{\\\"city\\\":", "[")
        .replace("\\\"Paris\\\"}", "1]");
    let mut stream =
        AnthropicStream::new([("weather".to_owned(), json!(true))].into_iter().collect())?;
    stream.push(array.as_bytes())?;
    assert!(matches!(
        stream.finish()?,
        Some(AnthropicOutcome::InvalidSchema { .. })
    ));
    let mut stream = AnthropicStream::new(std::collections::BTreeMap::new())?;
    stream.push(fixture.as_bytes())?;
    assert_eq!(
        stream.push(b"event: ping\ndata: {"),
        Err(StreamError::InvalidEvent)
    );
    assert_eq!(stream.finish(), Err(StreamError::InvalidEvent));
    Ok(())
}
