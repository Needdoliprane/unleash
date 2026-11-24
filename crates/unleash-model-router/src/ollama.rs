//! Bounded native Ollama Chat stream decoder.
//!
//! Wire references: <https://docs.ollama.com/api/chat>,
//! <https://docs.ollama.com/api/streaming>, <https://docs.ollama.com/api/errors>,
//! and <https://docs.ollama.com/capabilities/tool-calling>.
//! This decoder consumes a response body. Placement authority and dispatch
//! remain the caller's responsibility.

use crate::{MAX_EVENT, MAX_LINE, MAX_TOOLS, StreamError, Usage, compile_tool_contracts};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;
use unleash_protocol::from_json_str_strict;

/// A provider proposal awaiting separate broker authorization.
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct OllamaToolProposal {
    /// Provider identifier, when actually reported.
    pub id: Option<String>,
    /// Provider index, when actually reported.
    pub index: Option<u32>,
    /// Name matched against the caller's contract.
    pub name: String,
    /// Complete arguments validated against that contract.
    pub arguments: Value,
}

/// Terminal observation for one native Chat response.
#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OllamaOutcome {
    /// A complete response at a normal stop.
    Completed {
        /// Assistant content in stream order.
        text: String,
        /// Validated proposals in stream order.
        tools: Vec<OllamaToolProposal>,
        /// Counters reported by the terminal event.
        usage: Usage,
    },
    /// The provider reports output truncated by a limit.
    Truncated {
        /// Counters reported by the terminal event.
        usage: Usage,
    },
    /// A proposal has no matching schema or fails it.
    InvalidSchema {
        /// Counters reported by the terminal event.
        usage: Usage,
    },
    /// The provider sent a terminal error record.
    Failed {
        /// Provider status when present. The raw message is discarded.
        status: Option<u16>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Frame {
    Chat(Box<ChatFrame>),
    Error(ErrorFrame),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatFrame {
    model: String,
    #[serde(rename = "created_at")]
    _created_at: String,
    message: Message,
    done: bool,
    done_reason: Option<StopReason>,
    prompt_eval_count: Option<u64>,
    eval_count: Option<u64>,
    prompt_eval_cached_count: Option<u64>,
    #[serde(rename = "total_duration")]
    _total_duration: Option<u64>,
    #[serde(rename = "load_duration")]
    _load_duration: Option<u64>,
    #[serde(rename = "prompt_eval_duration")]
    _prompt_eval_duration: Option<u64>,
    #[serde(rename = "eval_duration")]
    _eval_duration: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum StopReason {
    Stop,
    Length,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorFrame {
    error: String,
    status: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    role: String,
    content: String,
    #[serde(rename = "thinking")]
    _thinking: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCall {
    id: Option<String>,
    #[serde(rename = "type")]
    _kind: Option<ToolKind>,
    function: ToolFunction,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolKind {
    Function,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolFunction {
    index: Option<u32>,
    name: String,
    arguments: Map<String, Value>,
}

enum Terminal {
    End(StopReason, Usage),
    Error(Option<u16>),
}

/// Decoder for one NDJSON response bound to the exact requested model name.
pub struct OllamaStream {
    model: String,
    schemas: BTreeMap<String, jsonschema::Validator>,
    line: Vec<u8>,
    received: usize,
    text: String,
    tools: Vec<OllamaToolProposal>,
    terminal: Option<Terminal>,
    failed: bool,
}

impl fmt::Debug for OllamaStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OllamaStream")
            .finish_non_exhaustive()
    }
}

impl OllamaStream {
    /// Create a decoder for an expected model and the caller's tool contracts.
    ///
    /// # Errors
    /// Returns an error for an invalid model identifier or tool contract.
    pub fn new(model: &str, contracts: BTreeMap<String, Value>) -> Result<Self, StreamError> {
        if model.is_empty()
            || model.len() > 256
            || model.trim() != model
            || model.chars().any(char::is_control)
        {
            return Err(StreamError::InvalidEvent);
        }
        Ok(Self {
            model: model.to_owned(),
            schemas: compile_tool_contracts(contracts)?,
            line: Vec::new(),
            received: 0,
            text: String::new(),
            tools: Vec::new(),
            terminal: None,
            failed: false,
        })
    }

    /// Consume a byte fragment without releasing content or proposals.
    ///
    /// # Errors
    /// Invalid or excessive input permanently fails this decoder.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        if self.failed {
            return Err(StreamError::InvalidEvent);
        }
        let result = self.push_inner(bytes);
        self.failed = result.is_err();
        result
    }

    fn push_inner(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        self.received = self.received.saturating_add(bytes.len());
        if self.received > MAX_EVENT {
            return Err(StreamError::LimitExceeded);
        }
        for segment in bytes.split_inclusive(|byte| *byte == b'\n') {
            if self.terminal.is_some() {
                return Err(StreamError::InvalidEvent);
            }
            let before_lf = segment.strip_suffix(b"\n");
            let payload = before_lf.unwrap_or(segment);
            if self.line.len().saturating_add(payload.len()) > MAX_LINE {
                return Err(StreamError::LimitExceeded);
            }
            self.line.extend_from_slice(payload);
            if before_lf.is_some() {
                let mut line = std::mem::take(&mut self.line);
                let result = self.accept_line(&line);
                line.clear();
                self.line = line;
                result?;
            }
        }
        Ok(())
    }

    fn accept_line(&mut self, line: &[u8]) -> Result<(), StreamError> {
        let frame = std::str::from_utf8(line).map_err(|_| StreamError::InvalidEvent)?;
        match from_json_str_strict(frame).map_err(|_| StreamError::InvalidEvent)? {
            Frame::Chat(frame) => self.accept_chat(*frame),
            Frame::Error(frame) => {
                if frame.error.trim().is_empty()
                    || frame
                        .status
                        .is_some_and(|status| !(400..=599).contains(&status))
                {
                    return Err(StreamError::InvalidEvent);
                }
                self.terminal = Some(Terminal::Error(frame.status));
                Ok(())
            }
        }
    }

    fn accept_chat(&mut self, frame: ChatFrame) -> Result<(), StreamError> {
        if frame.model != self.model
            || frame.message.role != "assistant"
            || frame.done != frame.done_reason.is_some()
        {
            return Err(StreamError::InvalidEvent);
        }
        if self
            .tools
            .len()
            .saturating_add(frame.message.tool_calls.len())
            > MAX_TOOLS
        {
            return Err(StreamError::LimitExceeded);
        }
        for tool in frame.message.tool_calls {
            if tool.function.name.trim().is_empty()
                || tool.function.name.len() > 1024
                || tool.id.as_ref().is_some_and(|id| {
                    id.trim().is_empty()
                        || id.len() > 1024
                        || self
                            .tools
                            .iter()
                            .any(|previous| previous.id.as_ref() == Some(id))
                })
            {
                return Err(StreamError::InvalidEvent);
            }
            self.tools.push(OllamaToolProposal {
                id: tool.id,
                index: tool.function.index,
                name: tool.function.name,
                arguments: Value::Object(tool.function.arguments),
            });
        }
        self.text.push_str(&frame.message.content);
        if let Some(reason) = frame.done_reason {
            self.terminal = Some(Terminal::End(
                reason,
                Usage {
                    input: frame.prompt_eval_count,
                    output: frame.eval_count,
                    cache: frame.prompt_eval_cached_count,
                    reasoning: None,
                },
            ));
        }
        Ok(())
    }

    /// Consume the decoder after the caller observed the end of the body.
    ///
    /// # Errors
    /// A failed decoder, partial line or absent terminal event yields an error.
    pub fn finish(self) -> Result<OllamaOutcome, StreamError> {
        if self.failed {
            return Err(StreamError::InvalidEvent);
        }
        if !self.line.is_empty() {
            return Err(StreamError::Interrupted);
        }
        match self.terminal.ok_or(StreamError::Interrupted)? {
            Terminal::End(StopReason::Stop, usage) => {
                if self.tools.iter().any(|proposal| {
                    !self
                        .schemas
                        .get(&proposal.name)
                        .is_some_and(|schema| schema.is_valid(&proposal.arguments))
                }) {
                    return Ok(OllamaOutcome::InvalidSchema { usage });
                }
                Ok(OllamaOutcome::Completed {
                    text: self.text,
                    tools: self.tools,
                    usage,
                })
            }
            Terminal::End(StopReason::Length, usage) => Ok(OllamaOutcome::Truncated { usage }),
            Terminal::Error(status) => Ok(OllamaOutcome::Failed { status }),
        }
    }
}
