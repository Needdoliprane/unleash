//! Partial T-F012-014/017 coverage on synthetic native Ollama NDJSON.

use crate::ollama::{OllamaOutcome, OllamaStream, OllamaToolProposal};
use crate::{MAX_EVENT, MAX_LINE, StreamError, Usage};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MODEL: &str = "fixture-chat:v1";

fn contracts() -> BTreeMap<String, Value> {
    BTreeMap::from([(
        "read_note".into(),
        json!({
            "type": "object",
            "required": ["id"],
            "properties": {"id": {"type":"integer"}},
            "additionalProperties": false
        }),
    )])
}

fn chat(message: &Value, terminal: Option<&str>) -> String {
    let mut frame = json!({
        "model": MODEL,
        "created_at": "2026-11-20T00:00:00Z",
        "message": message,
        "done": terminal.is_some(),
    });
    if let Some(reason) = terminal
        && let Some(fields) = frame.as_object_mut()
    {
        fields.insert("done_reason".into(), json!(reason));
        fields.insert("prompt_eval_count".into(), json!(0));
        fields.insert("eval_count".into(), json!(3));
    }
    format!("{frame}\n")
}

#[test]
fn t_f012_014_ollama_chat_is_invariant_under_byte_splits() -> Result<(), StreamError> {
    let first = chat(
        &json!({"role":"assistant","content":"Caf","thinking":"internal"}),
        None,
    );
    let end = chat(
        &json!({
            "role":"assistant",
            "content":"é",
            "tool_calls":[{"function":{"index":0,"name":"read_note","arguments":{"id":7}}}]
        }),
        Some("stop"),
    );
    let fixture = format!("{first}{end}").into_bytes();
    for split in 0..=fixture.len() {
        let (left, right) = fixture.split_at(split);
        let mut decoded = OllamaStream::new(MODEL, contracts())?;
        decoded.push(left)?;
        decoded.push(right)?;
        assert_eq!(
            decoded.finish()?,
            OllamaOutcome::Completed {
                text: "Café".into(),
                tools: vec![OllamaToolProposal {
                    id: None,
                    index: Some(0),
                    name: "read_note".into(),
                    arguments: json!({"id":7}),
                }],
                usage: Usage {
                    input: Some(0),
                    output: Some(3),
                    cache: None,
                    reasoning: None,
                },
            },
            "split {split}"
        );
    }
    Ok(())
}

#[test]
fn t_f012_014_ollama_requires_terminal_line_delimiter() -> Result<(), StreamError> {
    let progress = chat(&json!({"role":"assistant","content":"draft"}), None);
    let terminal = chat(&json!({"role":"assistant","content":""}), Some("stop"));
    for bytes in [progress.as_bytes(), terminal.trim_end().as_bytes()] {
        let mut decoded = OllamaStream::new(MODEL, BTreeMap::new())?;
        decoded.push(bytes)?;
        assert_eq!(decoded.finish(), Err(StreamError::Interrupted));
    }
    Ok(())
}

#[test]
fn t_f012_014_ollama_proposals_require_schema_and_normal_stop() -> Result<(), StreamError> {
    let message = json!({
        "role":"assistant",
        "content":"",
        "tool_calls":[{"id":"fixture-item","function":{"index":0,"name":"read_note","arguments":{"id":"wrong"}}}]
    });
    for reason in ["stop", "length"] {
        let mut decoded = OllamaStream::new(MODEL, contracts())?;
        decoded.push(chat(&message, Some(reason)).as_bytes())?;
        let usage = Usage {
            input: Some(0),
            output: Some(3),
            cache: None,
            reasoning: None,
        };
        let expected = if reason == "stop" {
            OllamaOutcome::InvalidSchema { usage }
        } else {
            OllamaOutcome::Truncated { usage }
        };
        assert_eq!(decoded.finish(), Ok(expected));
    }
    Ok(())
}

#[test]
fn t_f012_014_ollama_rejects_pairing_and_trailing_data() -> Result<(), StreamError> {
    let terminal = chat(&json!({"role":"assistant","content":""}), Some("stop"));
    let different_model = terminal.replace(MODEL, "different-model");
    let duplicate_key = terminal.replace("\"done\":true", "\"done\":false,\"done\":true");
    for malformed in [different_model, duplicate_key] {
        let mut decoded = OllamaStream::new(MODEL, BTreeMap::new())?;
        assert_eq!(
            decoded.push(malformed.as_bytes()),
            Err(StreamError::InvalidEvent)
        );
        assert_eq!(decoded.finish(), Err(StreamError::InvalidEvent));
    }
    let mut decoded = OllamaStream::new(MODEL, BTreeMap::new())?;
    decoded.push(terminal.as_bytes())?;
    assert_eq!(decoded.push(b"\n"), Err(StreamError::InvalidEvent));
    assert_eq!(decoded.finish(), Err(StreamError::InvalidEvent));
    Ok(())
}

#[test]
fn t_f012_017_ollama_does_not_invent_missing_usage() -> Result<(), StreamError> {
    let mut decoded = OllamaStream::new(MODEL, BTreeMap::new())?;
    let terminal = chat(&json!({"role":"assistant","content":"ok"}), Some("stop"))
        .replace(",\"prompt_eval_count\":0", "");
    decoded.push(terminal.as_bytes())?;
    assert_eq!(
        decoded.finish()?,
        OllamaOutcome::Completed {
            text: "ok".into(),
            tools: vec![],
            usage: Usage {
                input: None,
                output: Some(3),
                cache: None,
                reasoning: None,
            },
        }
    );
    Ok(())
}

#[test]
fn t_f012_014_ollama_reports_stream_failure_without_raw_message() -> Result<(), StreamError> {
    let mut decoded = OllamaStream::new(MODEL, BTreeMap::new())?;
    decoded.push(chat(&json!({"role":"assistant","content":"unfinished"}), None).as_bytes())?;
    decoded.push(b"{\"error\":\"fixture private diagnostic\",\"status\":500}\n")?;
    assert_eq!(
        decoded.finish(),
        Ok(OllamaOutcome::Failed { status: Some(500) })
    );
    Ok(())
}

#[test]
fn t_f012_014_ollama_bounds_frame_and_response() -> Result<(), StreamError> {
    let mut too_long = OllamaStream::new(MODEL, BTreeMap::new())?;
    assert_eq!(
        too_long.push(&vec![b' '; MAX_LINE + 1]),
        Err(StreamError::LimitExceeded)
    );

    let progress = chat(&json!({"role":"assistant","content":"part"}), None);
    let mut too_much = OllamaStream::new(MODEL, BTreeMap::new())?;
    for _ in 0..MAX_EVENT / progress.len() {
        too_much.push(progress.as_bytes())?;
    }
    assert_eq!(
        too_much.push(progress.as_bytes()),
        Err(StreamError::LimitExceeded)
    );
    assert_eq!(too_much.finish(), Err(StreamError::InvalidEvent));
    Ok(())
}
