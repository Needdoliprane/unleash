//! Restricted Anthropic Messages SSE adapter, specified from the official
//! streaming and stop-reason references consulted 2026-11-18. Fixture coverage
//! is not provider qualification; unsupported events fail closed.
//!
//! <https://platform.claude.com/docs/en/build-with-claude/streaming>
//! <https://platform.claude.com/docs/en/build-with-claude/handling-stop-reasons>

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Assembler, MAX_ARGUMENTS, MAX_EVENT, MAX_TOOLS, ModelOutcome, SseDecoder, StreamError,
    ToolProposal,
};

/// Counters remain absent when not supplied, including cache counters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AnthropicUsage {
    /// Input tokens, excluding separately reported cache categories.
    pub input: Option<u64>,
    /// Cumulative generated output tokens.
    pub output: Option<u64>,
    /// Tokens read from cache.
    pub cache_read: Option<u64>,
    /// Tokens written to cache.
    pub cache_creation: Option<u64>,
}

/// Successful stop conditions are distinct from one another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Natural end of turn.
    EndTurn,
    /// A caller-supplied stop sequence, as actually reported.
    StopSequence(String),
    /// One or more tool proposals were produced.
    ToolUse,
}

/// A finite observed outcome; proposals require separate broker authorization.
#[derive(Debug, PartialEq)]
pub enum AnthropicOutcome {
    /// Finished output and validated proposals.
    Success {
        /// Final text in block order.
        text: String,
        /// Schema-validated tool proposals.
        tools: Vec<ToolProposal>,
        /// Observed stop reason.
        stop: StopReason,
        /// Observed cumulative token counters.
        usage: AnthropicUsage,
    },
    /// Provider classified this result as a refusal.
    Refusal {
        /// Provider explanation if present.
        explanation: Option<String>,
        /// Counters when reported.
        usage: AnthropicUsage,
    },
    /// Generation reached a token or context-window limit.
    Truncated {
        /// Provider's exact stop reason.
        reason: String,
        /// Counters when reported.
        usage: AnthropicUsage,
    },
    /// Complete tool input is malformed, or lacks or violates its contract.
    InvalidSchema {
        /// Counters when reported.
        usage: AnthropicUsage,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireUsage {
    #[serde(rename = "input_tokens")]
    input: Option<u64>,
    #[serde(rename = "output_tokens")]
    output: Option<u64>,
    #[serde(rename = "cache_read_input_tokens")]
    cache_read: Option<u64>,
    #[serde(rename = "cache_creation_input_tokens")]
    cache_creation: Option<u64>,
}

impl AnthropicUsage {
    fn update(&mut self, u: WireUsage) -> Result<(), StreamError> {
        fn cumulative(old: &mut Option<u64>, new: Option<u64>) -> Result<(), StreamError> {
            if let Some(value) = new {
                if old.is_some_and(|previous| value < previous) {
                    return Err(StreamError::InvalidEvent);
                }
                *old = Some(value);
            }
            Ok(())
        }
        let mut next = self.clone();
        cumulative(&mut next.input, u.input)?;
        cumulative(&mut next.output, u.output)?;
        cumulative(&mut next.cache_read, u.cache_read)?;
        cumulative(&mut next.cache_creation, u.cache_creation)?;
        *self = next;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    role: String,
    content: Vec<Value>,
    model: String,
    stop_reason: Option<String>,
    stop_sequence: Option<String>,
    usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    Tool {
        id: String,
        name: String,
        input: Value,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum ContentDelta {
    #[serde(rename = "text_delta")]
    Text { text: String },
    #[serde(rename = "input_json_delta")]
    Input { partial_json: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StopDetails {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "category")]
    _category: Option<String>,
    explanation: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageDelta {
    #[serde(rename = "stop_reason")]
    reason: Option<String>,
    #[serde(rename = "stop_sequence")]
    sequence: Option<String>,
    #[serde(rename = "stop_details")]
    details: Option<StopDetails>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum Event {
    #[serde(rename = "message_start")]
    MessageStart { message: Message },
    #[serde(rename = "content_block_start")]
    BlockStart {
        index: u32,
        content_block: ContentBlock,
    },
    #[serde(rename = "content_block_delta")]
    BlockDelta { index: u32, delta: ContentDelta },
    #[serde(rename = "content_block_stop")]
    BlockStop { index: u32 },
    #[serde(rename = "message_delta")]
    MessageDelta {
        delta: MessageDelta,
        usage: Option<WireUsage>,
    },
    #[serde(rename = "message_stop")]
    MessageStop {},
    #[serde(rename = "ping")]
    Ping {},
}

#[derive(Debug)]
enum Block {
    Text {
        value: String,
        closed: bool,
    },
    Tool {
        id: String,
        size: usize,
        closed: bool,
    },
}
impl Block {
    fn closed(&self) -> bool {
        match self {
            Self::Text { closed, .. } | Self::Tool { closed, .. } => *closed,
        }
    }
}

/// Offline assembler for the explicitly supported Messages event subset.
#[derive(Debug)]
pub struct AnthropicStream {
    decoder: SseDecoder,
    tools: Assembler,
    blocks: Vec<Block>,
    usage: AnthropicUsage,
    started: bool,
    stop: Option<String>,
    stop_sequence: Option<String>,
    explanation: Option<String>,
    terminal: bool,
    pending: Option<AnthropicOutcome>,
    failed: bool,
}

impl AnthropicStream {
    /// Last fully accepted usage observation; absent counters remain unknown.
    /// Returns `None` until an entirely valid message start has been accepted.
    /// This is not a final result, a durable receipt, or confirmed billing.
    #[must_use]
    pub fn observed_usage(&self) -> Option<AnthropicUsage> {
        self.started.then(|| self.usage.clone())
    }

    /// Compile authorized tool names and their argument schemas.
    ///
    /// # Errors
    /// Invalid or excessive contracts are rejected before processing.
    pub fn new(contracts: BTreeMap<String, Value>) -> Result<Self, StreamError> {
        Ok(Self {
            decoder: SseDecoder::new(),
            tools: Assembler::new(contracts)?,
            blocks: Vec::new(),
            usage: AnthropicUsage::default(),
            started: false,
            stop: None,
            stop_sequence: None,
            explanation: None,
            terminal: false,
            pending: None,
            failed: false,
        })
    }

    /// Consume bytes without emitting results or executing any tools.
    ///
    /// # Errors
    /// Invalid framing, unsupported events, contradictory state and limits
    /// permanently poison this stream.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        if self.failed {
            return Err(StreamError::InvalidEvent);
        }
        let result = self.push_inner(bytes);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn push_inner(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        if bytes.len() > crate::MAX_CHUNK {
            return Err(StreamError::LimitExceeded);
        }
        for segment in bytes.split_inclusive(|byte| *byte == b'\r' || *byte == b'\n') {
            if self.terminal && !(self.decoder.skip_lf && segment == b"\n") {
                return Err(StreamError::InvalidEvent);
            }
            for raw in self.decoder.push(segment)? {
                if self.terminal {
                    return Err(StreamError::InvalidEvent);
                }
                let value: Value =
                    serde_json::from_str(&raw.data).map_err(|_| StreamError::InvalidEvent)?;
                if value.get("type").and_then(Value::as_str) != Some(raw.event.as_str()) {
                    return Err(StreamError::InvalidEvent);
                }
                let event: Event =
                    serde_json::from_str(&raw.data).map_err(|_| StreamError::InvalidEvent)?;
                self.apply(event)?;
            }
        }
        if self.terminal {
            self.decoder.finish()?;
        }
        Ok(())
    }

    fn apply(&mut self, event: Event) -> Result<(), StreamError> {
        match event {
            Event::MessageStart { message } => {
                if self.started
                    || message.id.is_empty()
                    || message.model.is_empty()
                    || message.kind != "message"
                    || message.role != "assistant"
                    || !message.content.is_empty()
                    || message.stop_reason.is_some()
                    || message.stop_sequence.is_some()
                {
                    return Err(StreamError::InvalidEvent);
                }
                if let Some(usage) = message.usage {
                    self.usage.update(usage)?;
                }
                self.started = true;
            }
            Event::Ping {} => {
                if !self.started {
                    return Err(StreamError::InvalidEvent);
                }
            }
            Event::BlockStart {
                index,
                content_block,
            } => self.block_start(index, content_block)?,
            Event::BlockDelta { index, delta } => self.block_delta(index, delta)?,
            Event::BlockStop { index } => self.block_stop(index)?,
            Event::MessageDelta { delta, usage } => self.message_delta(delta, usage)?,
            Event::MessageStop {} => self.message_stop()?,
        }
        Ok(())
    }

    fn block_start(&mut self, index: u32, block: ContentBlock) -> Result<(), StreamError> {
        if !self.started
            || self.stop.is_some()
            || self.blocks.len() >= MAX_TOOLS
            || u32::try_from(self.blocks.len()) != Ok(index)
            || self.blocks.last().is_some_and(|b| !b.closed())
        {
            return Err(StreamError::InvalidEvent);
        }
        let block = match block {
            ContentBlock::Text { text } if text.is_empty() => Block::Text { value: text, closed: false },
            ContentBlock::Tool { id, name, input } if !id.is_empty() && input.as_object().is_some_and(serde_json::Map::is_empty) && !self.blocks.iter().any(|block| matches!(block, Block::Tool { id: previous, .. } if previous == &id)) => {
                self.tools.accept(&json!({"type":"tool_start","index":index,"id":id,"name":name}).to_string())?;
                Block::Tool { id, size: 0, closed: false }
            }
            _ => return Err(StreamError::InvalidEvent),
        };
        self.blocks.push(block);
        Ok(())
    }

    fn block_delta(&mut self, index: u32, delta: ContentDelta) -> Result<(), StreamError> {
        let index_usize = usize::try_from(index).map_err(|_| StreamError::InvalidEvent)?;
        let block = self
            .blocks
            .get_mut(index_usize)
            .ok_or(StreamError::InvalidEvent)?;
        match (block, delta) {
            (
                Block::Text {
                    value,
                    closed: false,
                },
                ContentDelta::Text { text },
            ) => {
                if value.len().saturating_add(text.len()) > MAX_EVENT {
                    return Err(StreamError::LimitExceeded);
                }
                value.push_str(&text);
            }
            (
                Block::Tool {
                    size,
                    closed: false,
                    ..
                },
                ContentDelta::Input { partial_json },
            ) => {
                *size = size.saturating_add(partial_json.len());
                if *size > MAX_ARGUMENTS {
                    return Err(StreamError::LimitExceeded);
                }
                self.tools.accept(
                    &json!({"type":"tool_delta","index":index,"arguments":partial_json})
                        .to_string(),
                )?;
            }
            _ => return Err(StreamError::InvalidEvent),
        }
        Ok(())
    }

    fn block_stop(&mut self, index: u32) -> Result<(), StreamError> {
        let block = self
            .blocks
            .get_mut(usize::try_from(index).map_err(|_| StreamError::InvalidEvent)?)
            .ok_or(StreamError::InvalidEvent)?;
        match block {
            Block::Text { closed, .. } if !*closed => *closed = true,
            Block::Tool { size, closed, .. } if !*closed => {
                if *size == 0 {
                    self.tools.accept(
                        &json!({"type":"tool_delta","index":index,"arguments":"{}"}).to_string(),
                    )?;
                }
                self.tools
                    .accept(&json!({"type":"tool_end","index":index}).to_string())?;
                *closed = true;
            }
            _ => return Err(StreamError::InvalidEvent),
        }
        Ok(())
    }

    fn message_delta(
        &mut self,
        delta: MessageDelta,
        usage: Option<WireUsage>,
    ) -> Result<(), StreamError> {
        if !self.started || self.blocks.iter().any(|block| !block.closed()) {
            return Err(StreamError::InvalidEvent);
        }
        if let Some(reason) = delta.reason {
            if self
                .stop
                .as_ref()
                .is_some_and(|previous| previous != &reason)
            {
                return Err(StreamError::InvalidEvent);
            }
            self.stop = Some(reason);
        }
        if let Some(sequence) = delta.sequence {
            if self
                .stop_sequence
                .as_ref()
                .is_some_and(|previous| previous != &sequence)
            {
                return Err(StreamError::InvalidEvent);
            }
            self.stop_sequence = Some(sequence);
        }
        if let Some(details) = delta.details {
            if details.kind != "refusal" || self.stop.as_deref() != Some("refusal") {
                return Err(StreamError::InvalidEvent);
            }
            if self.explanation.is_some() && self.explanation != details.explanation {
                return Err(StreamError::InvalidEvent);
            }
            self.explanation = details.explanation;
        }
        if let Some(usage) = usage {
            self.usage.update(usage)?;
        }
        Ok(())
    }

    fn message_stop(&mut self) -> Result<(), StreamError> {
        if !self.started || self.blocks.iter().any(|block| !block.closed()) {
            return Err(StreamError::InvalidEvent);
        }
        let reason = self.stop.as_deref().ok_or(StreamError::InvalidEvent)?;
        let has_tool = self
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Tool { .. }));
        let usage = self.usage.clone();
        let result = match reason {
            "refusal" if self.stop_sequence.is_none() => AnthropicOutcome::Refusal {
                explanation: self.explanation.clone(),
                usage,
            },
            "max_tokens" | "model_context_window_exceeded"
                if self.stop_sequence.is_none() && self.explanation.is_none() =>
            {
                AnthropicOutcome::Truncated {
                    reason: reason.to_owned(),
                    usage,
                }
            }
            "end_turn" | "tool_use" | "stop_sequence" => {
                if self.explanation.is_some() {
                    return Err(StreamError::InvalidEvent);
                }
                let stop = match reason {
                    "end_turn" if !has_tool && self.stop_sequence.is_none() => StopReason::EndTurn,
                    "tool_use" if has_tool && self.stop_sequence.is_none() => StopReason::ToolUse,
                    "stop_sequence" if !has_tool => StopReason::StopSequence(
                        self.stop_sequence
                            .clone()
                            .ok_or(StreamError::InvalidEvent)?,
                    ),
                    _ => return Err(StreamError::InvalidEvent),
                };
                let mut text = String::new();
                for block in &self.blocks {
                    if let Block::Text { value, .. } = block {
                        if text.len().saturating_add(value.len()) > MAX_EVENT {
                            return Err(StreamError::LimitExceeded);
                        }
                        text.push_str(value);
                    }
                }
                match self.tools.accept(r#"{"type":"complete"}"#)? {
                    Some(ModelOutcome::Success { tools, .. })
                        if tools.iter().all(|tool| tool.arguments.is_object()) =>
                    {
                        AnthropicOutcome::Success {
                            text,
                            tools,
                            stop,
                            usage,
                        }
                    }
                    Some(ModelOutcome::Success { .. } | ModelOutcome::InvalidSchema) => {
                        AnthropicOutcome::InvalidSchema { usage }
                    }
                    _ => return Err(StreamError::InvalidEvent),
                }
            }
            _ => return Err(StreamError::InvalidEvent),
        };
        self.pending = Some(result);
        self.terminal = true;
        Ok(())
    }

    /// Declare real transport EOF and take a validated result at most once.
    ///
    /// # Errors
    /// Incomplete framing, absence of a terminal, or any prior error poisons
    /// the stream and yields no result.
    pub fn finish(&mut self) -> Result<Option<AnthropicOutcome>, StreamError> {
        if self.failed {
            return Err(StreamError::InvalidEvent);
        }
        if let Err(error) = self.decoder.finish() {
            self.failed = true;
            return Err(error);
        }
        if !self.terminal {
            self.failed = true;
            return Err(StreamError::Interrupted);
        }
        Ok(self.pending.take())
    }
}
