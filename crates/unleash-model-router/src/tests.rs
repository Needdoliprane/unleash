use crate::{Assembler, ModelOutcome, SseDecoder, StreamError, Usage};
use serde_json::json;

fn assembler(schema: serde_json::Value) -> Result<Assembler, StreamError> {
    Assembler::new(
        [
            ("weather".to_owned(), schema.clone()),
            ("n".to_owned(), schema),
        ]
        .into_iter()
        .collect(),
    )
}

// T-F012-004 / AC-F012-004 / REQ-MODEL-004: data is not an action.
#[test]
fn fragmented_tool_waits_for_terminal_and_schema_validation()
-> Result<(), Box<dyn std::error::Error>> {
    let mut decoder = SseDecoder::new();
    let mut assembler = assembler(
        json!({"type":"object","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false}),
    )?;
    let stream = concat!(
        "data: {\"type\":\"tool_start\",\"index\":0,\"id\":\"a\",\"name\":\"weather\"}\r\n\r\n",
        "data: {\"type\":\"tool_delta\",\"index\":0,\"arguments\":\"{\\\"city\\\":\\\"Parisé\\\"}\"}\r\n\r\n",
        "data: {\"type\":\"tool_end\",\"index\":0}\n\ndata: {\"type\":\"complete\"}\n\n"
    );
    let mut outcome = None;
    for byte in stream.as_bytes() {
        for event in decoder.push(std::slice::from_ref(byte))? {
            let result = assembler.accept(&event.data)?;
            if result.is_some() {
                outcome = result;
            }
        }
    }
    assert!(
        matches!(outcome, Some(ModelOutcome::Success { ref tools, .. }) if tools.len() == 1 && tools.first().is_some_and(|tool| tool.arguments == json!({"city":"Parisé"})))
    );
    decoder.finish()?;
    assert!(assembler.finish()?.is_none());
    Ok(())
}

#[test]
fn incomplete_unknown_oversized_and_duplicate_terminal_are_safe()
-> Result<(), Box<dyn std::error::Error>> {
    let mut d = SseDecoder::new();
    assert!(d.push(b"data: partial").is_ok());
    assert_eq!(d.finish(), Err(StreamError::Interrupted));
    let mut d = SseDecoder::new();
    assert_eq!(d.push(&vec![b'x'; 65_537]), Err(StreamError::LimitExceeded));
    let mut a = assembler(json!({"type":"object"}))?;
    assert_eq!(
        a.accept(r#"{"type":"complete","unknown":1}"#),
        Err(StreamError::InvalidEvent)
    );
    let mut a = assembler(json!({"type":"object"}))?;
    assert!(
        a.accept(r#"{"type":"tool_start","index":0,"id":"x","name":"n"}"#)?
            .is_none()
    );
    assert_eq!(a.finish(), Err(StreamError::Interrupted));
    let mut a = assembler(json!({"type":"object"}))?;
    assert!(a.accept(r#"{"type":"complete"}"#)?.is_some());
    assert_eq!(a.accept(r#"{"type":"complete"}"#)?, None);
    Ok(())
}

// T-F012-005/011: synthetic closed fixture dialect, no provider qualification.
#[test]
fn outcomes_and_absent_usage_are_distinct() -> Result<(), Box<dyn std::error::Error>> {
    for (input, expected) in [
        (r#"{"type":"refusal","reason":"no"}"#, "refusal"),
        (r#"{"type":"truncated"}"#, "truncated"),
    ] {
        let mut a = assembler(json!({"type":"object"}))?;
        let result = a.accept(input)?;
        assert!(matches!(
            (expected, result),
            ("refusal", Some(ModelOutcome::Refusal { .. }))
                | ("truncated", Some(ModelOutcome::Truncated))
        ));
    }
    let mut a = assembler(json!({"type":"object"}))?;
    assert_eq!(
        a.accept(r#"{"type":"complete","usage":{"input":0}}"#)?,
        Some(ModelOutcome::Success {
            tools: vec![],
            usage: Some(Usage {
                input: Some(0),
                output: None,
                cache: None,
                reasoning: None
            })
        })
    );
    let mut a = assembler(json!({"type":"object"}))?;
    assert_eq!(
        a.accept(r#"{"type":"complete"}"#)?,
        Some(ModelOutcome::Success {
            tools: vec![],
            usage: None
        })
    );
    let mut a = assembler(json!({"type":"object","required":["x"]}))?;
    a.accept(r#"{"type":"tool_start","index":0,"id":"x","name":"n"}"#)?;
    a.accept(r#"{"type":"tool_delta","index":0,"arguments":"{}"}"#)?;
    a.accept(r#"{"type":"tool_end","index":0}"#)?;
    assert_eq!(
        a.accept(r#"{"type":"complete"}"#)?,
        Some(ModelOutcome::InvalidSchema)
    );
    Ok(())
}

#[test]
fn sse_framing_is_invariant_under_every_two_chunk_split() -> Result<(), Box<dyn std::error::Error>>
{
    let input = "\u{feff}: comment\r\nevent: custom\r\ndata: café\r\ndata: second\r\n\r\ndata\n\n"
        .as_bytes();
    let expected = vec![
        crate::SseEvent {
            event: "custom".into(),
            data: "café\nsecond".into(),
        },
        crate::SseEvent {
            event: "message".into(),
            data: String::new(),
        },
    ];
    for split in 0..=input.len() {
        let mut parser = SseDecoder::new();
        let mut events = parser.push(input.get(..split).ok_or(StreamError::InvalidEvent)?)?;
        events.extend(parser.push(input.get(split..).ok_or(StreamError::InvalidEvent)?)?);
        parser.finish()?;
        assert_eq!(events, expected, "split {split}");
    }
    Ok(())
}

#[test]
fn invalid_tool_outputs_and_excess_arguments_never_produce_proposals()
-> Result<(), Box<dyn std::error::Error>> {
    for (name, arguments) in [("unknown", "{}"), ("n", "{"), ("n", "[]")] {
        let mut a = assembler(json!({"type":"object"}))?;
        a.accept(&json!({"type":"tool_start","index":0,"id":"x","name":name}).to_string())?;
        a.accept(&json!({"type":"tool_delta","index":0,"arguments":arguments}).to_string())?;
        a.accept(r#"{"type":"tool_end","index":0}"#)?;
        assert_eq!(
            a.accept(r#"{"type":"complete"}"#)?,
            Some(ModelOutcome::InvalidSchema)
        );
    }
    let mut a = assembler(json!({"type":"object"}))?;
    a.accept(r#"{"type":"tool_start","index":0,"id":"x","name":"n"}"#)?;
    let event = json!({"type":"tool_delta","index":0,"arguments":"x".repeat(262_145)}).to_string();
    assert_eq!(a.accept(&event), Err(StreamError::LimitExceeded));
    assert_eq!(
        a.accept(r#"{"type":"complete"}"#),
        Err(StreamError::InvalidEvent)
    );
    let mut a = assembler(json!({"type":"object"}))?;
    assert_eq!(
        a.accept(r#"{"type":"complete","usage":{"input":0,"extra":1}}"#),
        Err(StreamError::InvalidEvent)
    );
    Ok(())
}

#[test]
fn interruption_mid_json_and_unfinished_tool_do_not_succeed()
-> Result<(), Box<dyn std::error::Error>> {
    let mut d = SseDecoder::new();
    assert!(d.push(b"data: {\"type\":\"complete\"")?.is_empty());
    assert_eq!(d.finish(), Err(StreamError::Interrupted));
    let mut a = assembler(json!({"type":"object"}))?;
    a.accept(r#"{"type":"tool_start","index":0,"id":"x","name":"n"}"#)?;
    a.accept(r#"{"type":"tool_delta","index":0,"arguments":"{}"}"#)?;
    assert_eq!(
        a.accept(r#"{"type":"complete"}"#),
        Err(StreamError::Interrupted)
    );
    assert_eq!(a.finish(), Err(StreamError::InvalidEvent));
    Ok(())
}
