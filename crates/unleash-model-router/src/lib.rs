//! Bounded SSE framing and normalization of a synthetic, closed fixture dialect.
//!
//! This crate performs no transport, provider qualification, authorization, or
//! tool execution. The limits below bound a single chunk, line, event, tool
//! list and argument buffer; they are local implementation limits, not provider
//! capabilities. A real adapter must translate its qualified dialect explicitly.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

/// Bounded, explicitly restricted Anthropic Messages SSE dialect.
pub mod anthropic;
/// Bounded, explicitly restricted OpenAI Responses SSE dialect.
pub mod openai;

#[cfg(test)]
mod anthropic_tests;
#[cfg(test)]
mod http_fixture_tests;
#[cfg(test)]
mod openai_tests;
#[cfg(test)]
mod qualification_http;

const MAX_CHUNK: usize = 1_048_576;
const MAX_LINE: usize = 65_536;
const MAX_EVENT: usize = 1_048_576;
const MAX_TOOLS: usize = 64;
const MAX_ARGUMENTS: usize = 262_144;

/// A framing or closed-dialect error. None authorizes a retry or action.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StreamError {
    /// A configured memory bound was exceeded.
    #[error("stream limit exceeded")]
    LimitExceeded,
    /// Input is not valid UTF-8 or a valid closed fixture event.
    #[error("invalid stream event")]
    InvalidEvent,
    /// The input ended before a complete event or terminal result.
    #[error("stream interrupted")]
    Interrupted,
    /// The caller's tool contract could not be compiled.
    #[error("invalid tool contract")]
    InvalidContract,
}

/// A dispatched SSE event. IDs and retry hints do not affect normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The event field, defaulting to `message`.
    pub event: String,
    /// Data fields joined with newlines.
    pub data: String,
}

/// Incremental, byte-oriented SSE parser with bounded buffers.
#[derive(Debug)]
pub struct SseDecoder {
    line: Vec<u8>,
    data: String,
    event: String,
    has_data: bool,
    skip_lf: bool,
    first_line: Option<()>,
    failed: bool,
}

impl Default for SseDecoder {
    fn default() -> Self {
        Self {
            line: Vec::new(),
            data: String::new(),
            event: String::new(),
            has_data: false,
            skip_lf: false,
            first_line: Some(()),
            failed: false,
        }
    }
}

impl SseDecoder {
    /// Create a parser for one stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume one bounded byte chunk and return only fully delimited events.
    ///
    /// # Errors
    /// Invalid UTF-8, excessive input or an earlier error poison the parser.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, StreamError> {
        if self.failed {
            return Err(StreamError::InvalidEvent);
        }
        let result = self.push_inner(chunk);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn push_inner(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, StreamError> {
        if chunk.len() > MAX_CHUNK {
            return Err(StreamError::LimitExceeded);
        }
        let mut events = Vec::new();
        for &byte in chunk {
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\r' || byte == b'\n' {
                self.skip_lf = byte == b'\r';
                if let Some(event) = self.end_line()? {
                    events.push(event);
                }
            } else {
                if self.line.len() == MAX_LINE {
                    return Err(StreamError::LimitExceeded);
                }
                self.line.push(byte);
            }
        }
        Ok(events)
    }

    fn end_line(&mut self) -> Result<Option<SseEvent>, StreamError> {
        let bytes = std::mem::take(&mut self.line);
        let mut line = std::str::from_utf8(&bytes).map_err(|_| StreamError::InvalidEvent)?;
        if self.first_line.take().is_some() {
            line = line.strip_prefix('\u{feff}').unwrap_or(line);
        }
        if line.is_empty() {
            let event = if self.has_data {
                self.data.pop(); // each data field appends exactly one newline
                Some(SseEvent {
                    event: if self.event.is_empty() {
                        "message".into()
                    } else {
                        std::mem::take(&mut self.event)
                    },
                    data: std::mem::take(&mut self.data),
                })
            } else {
                None
            };
            self.event.clear();
            self.has_data = false;
            return Ok(event);
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                if self
                    .data
                    .len()
                    .saturating_add(value.len())
                    .saturating_add(1)
                    > MAX_EVENT
                {
                    return Err(StreamError::LimitExceeded);
                }
                self.data.push_str(value);
                self.data.push('\n');
                self.has_data = true;
            }
            "event" => value.clone_into(&mut self.event),
            _ => {}
        }
        Ok(None)
    }

    /// Check that the stream ended at an event boundary.
    ///
    /// # Errors
    /// A partial line/event or a previously failed parser is not accepted.
    pub fn finish(&self) -> Result<(), StreamError> {
        if self.failed {
            return Err(StreamError::InvalidEvent);
        }
        if !self.line.is_empty() || self.has_data || !self.event.is_empty() {
            return Err(StreamError::Interrupted);
        }
        Ok(())
    }
}

/// Usage counters are individually optional: absence is not zero.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// Input tokens, when reported.
    pub input: Option<u64>,
    /// Output tokens, when reported.
    pub output: Option<u64>,
    /// Cached tokens, when reported.
    pub cache: Option<u64>,
    /// Reasoning tokens, when reported.
    pub reasoning: Option<u64>,
}

/// A validated proposal, not an authorized or dispatched tool action.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolProposal {
    /// Provider-side item identifier for correlation.
    pub id: String,
    /// Name matched against the caller's tool contract.
    pub name: String,
    /// Parsed arguments satisfying the supplied JSON Schema.
    pub arguments: Value,
}

/// Terminal result of the synthetic stream.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelOutcome {
    /// Completed with proposals and optional usage.
    Success {
        /// Validated tool proposals.
        tools: Vec<ToolProposal>,
        /// Reported usage, if present.
        usage: Option<Usage>,
    },
    /// A provider refusal.
    Refusal {
        /// Refusal text.
        reason: String,
    },
    /// An explicitly truncated result.
    Truncated,
    /// Complete output that is not valid JSON or violates the tool contract.
    InvalidSchema,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum FixtureEvent {
    ToolStart {
        index: u32,
        id: String,
        name: String,
    },
    ToolDelta {
        index: u32,
        arguments: String,
    },
    ToolEnd {
        index: u32,
    },
    Complete {
        usage: Option<Usage>,
    },
    Refusal {
        reason: String,
    },
    Truncated,
}

#[derive(Debug)]
struct PendingTool {
    id: String,
    name: String,
    arguments: String,
    ended: bool,
}

/// Assembles only the documented synthetic fixture dialect; it never executes tools.
#[derive(Debug)]
pub struct Assembler {
    schemas: BTreeMap<String, jsonschema::Validator>,
    tools: BTreeMap<u32, PendingTool>,
    terminal: Option<String>,
    failed: bool,
}

impl Assembler {
    /// Compile the JSON Schema contract for each explicitly named tool.
    ///
    /// # Errors
    /// Returns `InvalidContract` if the schema cannot be compiled.
    pub fn new(contracts: BTreeMap<String, Value>) -> Result<Self, StreamError> {
        if contracts.len() > MAX_TOOLS {
            return Err(StreamError::LimitExceeded);
        }
        let mut schemas = BTreeMap::new();
        for (name, schema) in contracts {
            if name.is_empty() || name.len() > 1024 {
                return Err(StreamError::InvalidContract);
            }
            schemas.insert(
                name,
                jsonschema::validator_for(&schema).map_err(|_| StreamError::InvalidContract)?,
            );
        }
        Ok(Self {
            schemas,
            tools: BTreeMap::new(),
            terminal: None,
            failed: false,
        })
    }

    /// Accept one complete event payload; a result is emitted at most once.
    ///
    /// # Errors
    /// Invalid order, duplicate IDs, unknown fields or exceeded bounds poison
    /// the assembler. A byte-identical repeated terminal is ignored.
    pub fn accept(&mut self, payload: &str) -> Result<Option<ModelOutcome>, StreamError> {
        if self.failed {
            return Err(StreamError::InvalidEvent);
        }
        if let Some(terminal) = &self.terminal {
            if payload == terminal {
                return Ok(None);
            }
            self.failed = true;
            return Err(StreamError::InvalidEvent);
        }
        let result = self.accept_inner(payload);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn accept_inner(&mut self, payload: &str) -> Result<Option<ModelOutcome>, StreamError> {
        if payload.len() > MAX_EVENT {
            return Err(StreamError::LimitExceeded);
        }
        let event: FixtureEvent =
            serde_json::from_str(payload).map_err(|_| StreamError::InvalidEvent)?;
        let outcome = match event {
            FixtureEvent::ToolStart { index, id, name } => {
                if self.tools.len() >= MAX_TOOLS {
                    return Err(StreamError::LimitExceeded);
                }
                if id.is_empty()
                    || id.len() > 1024
                    || name.is_empty()
                    || name.len() > 1024
                    || self.tools.contains_key(&index)
                    || self.tools.values().any(|tool| tool.id == id)
                {
                    return Err(StreamError::InvalidEvent);
                }
                self.tools.insert(
                    index,
                    PendingTool {
                        id,
                        name,
                        arguments: String::new(),
                        ended: false,
                    },
                );
                None
            }
            FixtureEvent::ToolDelta { index, arguments } => {
                let tool = self
                    .tools
                    .get_mut(&index)
                    .ok_or(StreamError::InvalidEvent)?;
                if tool.ended {
                    return Err(StreamError::InvalidEvent);
                }
                if tool.arguments.len().saturating_add(arguments.len()) > MAX_ARGUMENTS {
                    return Err(StreamError::LimitExceeded);
                }
                tool.arguments.push_str(&arguments);
                None
            }
            FixtureEvent::ToolEnd { index } => {
                let tool = self
                    .tools
                    .get_mut(&index)
                    .ok_or(StreamError::InvalidEvent)?;
                if tool.ended {
                    return Err(StreamError::InvalidEvent);
                }
                tool.ended = true;
                None
            }
            FixtureEvent::Complete { usage } => {
                let mut proposals = Vec::with_capacity(self.tools.len());
                for tool in self.tools.values() {
                    if !tool.ended {
                        return Err(StreamError::Interrupted);
                    }
                    let Ok(arguments) = serde_json::from_str::<Value>(&tool.arguments) else {
                        self.terminal = Some(payload.to_owned());
                        return Ok(Some(ModelOutcome::InvalidSchema));
                    };
                    if !self
                        .schemas
                        .get(&tool.name)
                        .is_some_and(|schema| schema.is_valid(&arguments))
                    {
                        self.terminal = Some(payload.to_owned());
                        return Ok(Some(ModelOutcome::InvalidSchema));
                    }
                    proposals.push(ToolProposal {
                        id: tool.id.clone(),
                        name: tool.name.clone(),
                        arguments,
                    });
                }
                Some(ModelOutcome::Success {
                    tools: proposals,
                    usage,
                })
            }
            FixtureEvent::Refusal { reason } => Some(ModelOutcome::Refusal { reason }),
            FixtureEvent::Truncated => Some(ModelOutcome::Truncated),
        };
        if outcome.is_some() {
            self.terminal = Some(payload.to_owned());
        }
        Ok(outcome)
    }

    /// Confirm that a terminal result was observed; it is not emitted again.
    ///
    /// # Errors
    /// An incomplete or failed stream is not a successful result.
    pub fn finish(&self) -> Result<Option<ModelOutcome>, StreamError> {
        if self.failed {
            return Err(StreamError::InvalidEvent);
        }
        if self.terminal.is_none() {
            return Err(StreamError::Interrupted);
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
