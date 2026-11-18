//! Restricted OpenAI Responses SSE adapter, based on the streaming-events
//! reference consulted 2026-11-18. Unsupported fields and event types fail
//! closed. This is fixture coverage, not live provider qualification.
//!
//! <https://developers.openai.com/api/reference/resources/responses/streaming-events>

use crate::{Assembler, MAX_EVENT, MAX_TOOLS, ModelOutcome, SseDecoder, StreamError, ToolProposal};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Reported token counters; detail fields remain absent when not provided.
#[derive(Debug, PartialEq)]
pub struct OpenAiUsage {
    /// Input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Total tokens reported by the provider.
    pub total: u64,
    /// Cached input tokens, if reported.
    pub cache: Option<u64>,
    /// Cache write tokens, if reported.
    pub cache_write: Option<u64>,
    /// Reasoning tokens, if reported.
    pub reasoning: Option<u64>,
}

/// A completed Responses result, distinct from provider availability errors.
#[derive(Debug, PartialEq)]
pub enum OpenAiOutcome {
    /// Validated response; proposals still require broker authorization.
    Success {
        /// Final text in output order.
        text: String,
        /// Validated tool proposals.
        tools: Vec<ToolProposal>,
        /// Provider-reported counters when present.
        usage: Option<OpenAiUsage>,
    },
    /// Refusal received and finalized by the provider.
    Refusal {
        /// Refusal content.
        reason: String,
        /// Counters when reported.
        usage: Option<OpenAiUsage>,
    },
    /// Provider reports incomplete generation.
    Truncated {
        /// Counters when reported.
        usage: Option<OpenAiUsage>,
    },
    /// Final tool arguments violate or lack the applicable contract.
    InvalidSchema {
        /// Counters when reported.
        usage: Option<OpenAiUsage>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", deny_unknown_fields)]
enum Item {
    #[serde(rename = "message")]
    Message {
        id: String,
        status: String,
        role: String,
        content: Vec<Part>,
    },
    #[serde(rename = "function_call")]
    Function {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
        status: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", deny_unknown_fields)]
enum Part {
    #[serde(rename = "output_text")]
    Text {
        text: String,
        annotations: Vec<Value>,
        logprobs: Vec<Value>,
    },
    #[serde(rename = "refusal")]
    Refusal { refusal: String },
}

impl Part {
    fn content(&self) -> Result<&str, StreamError> {
        match self {
            Self::Text {
                text,
                annotations,
                logprobs,
            } if annotations.is_empty() && logprobs.is_empty() => Ok(text),
            Self::Refusal { refusal } => Ok(refusal),
            Self::Text { .. } => Err(StreamError::InvalidEvent),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputDetail {
    cached_tokens: u64,
    cache_write_tokens: Option<u64>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputDetail {
    reasoning_tokens: u64,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireUsage {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    input_tokens_details: Option<InputDetail>,
    output_tokens_details: Option<OutputDetail>,
}

impl From<WireUsage> for OpenAiUsage {
    fn from(u: WireUsage) -> Self {
        Self {
            input: u.input_tokens,
            output: u.output_tokens,
            total: u.total_tokens,
            cache: u.input_tokens_details.as_ref().map(|d| d.cached_tokens),
            cache_write: u.input_tokens_details.and_then(|d| d.cache_write_tokens),
            reasoning: u.output_tokens_details.map(|d| d.reasoning_tokens),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IncompleteDetails {
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reasoning {
    #[serde(rename = "effort")]
    _effort: Option<String>,
    #[serde(rename = "summary")]
    _summary: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextSettings {
    format: TextFormat,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextFormat {
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolDefinition {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "name")]
    _name: String,
    #[serde(rename = "description")]
    _description: Option<String>,
    #[serde(rename = "parameters")]
    _parameters: Value,
    #[serde(rename = "strict")]
    _strict: Option<bool>,
}

// Only this documented response-object subset is supported. Unrecognized
// fields and nested variants fail closed rather than being silently projected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    id: String,
    object: String,
    created_at: f64,
    status: String,
    output: Vec<Item>,
    usage: Option<WireUsage>,
    incomplete_details: Option<IncompleteDetails>,
    #[serde(rename = "input")]
    _input: Option<Vec<()>>,
    #[serde(rename = "access_programs")]
    _access_programs: Option<()>,
    #[serde(rename = "completed_at")]
    _completed_at: Option<f64>,
    #[serde(rename = "error")]
    _error: Option<()>,
    #[serde(rename = "instructions")]
    _instructions: Option<String>,
    #[serde(rename = "max_output_tokens")]
    _max_output_tokens: Option<u64>,
    #[serde(rename = "model")]
    _model: Option<String>,
    #[serde(rename = "parallel_tool_calls")]
    _parallel_tool_calls: Option<bool>,
    #[serde(rename = "previous_response_id")]
    _previous_response_id: Option<String>,
    #[serde(rename = "reasoning")]
    _reasoning: Option<Reasoning>,
    #[serde(rename = "reasoning_effort")]
    _reasoning_effort: Option<String>,
    #[serde(rename = "store")]
    _store: Option<bool>,
    #[serde(rename = "temperature")]
    _temperature: Option<f64>,
    text: Option<TextSettings>,
    #[serde(rename = "tool_choice")]
    _tool_choice: Option<String>,
    tools: Option<Vec<ToolDefinition>>,
    #[serde(rename = "top_p")]
    _top_p: Option<f64>,
    #[serde(rename = "truncation")]
    _truncation: Option<String>,
    #[serde(rename = "user")]
    _user: Option<String>,
    #[serde(rename = "metadata")]
    _metadata: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
enum Event {
    #[serde(rename = "response.created")]
    Created {
        sequence_number: u64,
        response: Response,
    },
    #[serde(rename = "response.in_progress")]
    InProgress {
        sequence_number: u64,
        response: Response,
    },
    #[serde(rename = "response.completed")]
    Completed {
        sequence_number: u64,
        response: Response,
    },
    #[serde(rename = "response.incomplete")]
    Incomplete {
        sequence_number: u64,
        response: Response,
    },
    #[serde(rename = "response.output_item.added")]
    ItemAdded {
        sequence_number: u64,
        output_index: u32,
        item: Item,
    },
    #[serde(rename = "response.output_item.done")]
    ItemDone {
        sequence_number: u64,
        output_index: u32,
        item: Item,
    },
    #[serde(rename = "response.content_part.added")]
    PartAdded {
        sequence_number: u64,
        output_index: u32,
        item_id: String,
        content_index: u32,
        part: Part,
    },
    #[serde(rename = "response.content_part.done")]
    PartDone {
        sequence_number: u64,
        output_index: u32,
        item_id: String,
        content_index: u32,
        part: Part,
    },
    #[serde(rename = "response.output_text.delta")]
    TextDelta {
        sequence_number: u64,
        output_index: u32,
        item_id: String,
        content_index: u32,
        delta: String,
        logprobs: Vec<Value>,
    },
    #[serde(rename = "response.output_text.done")]
    TextDone {
        sequence_number: u64,
        output_index: u32,
        item_id: String,
        content_index: u32,
        text: String,
        logprobs: Vec<Value>,
    },
    #[serde(rename = "response.refusal.delta")]
    RefusalDelta {
        sequence_number: u64,
        output_index: u32,
        item_id: String,
        content_index: u32,
        delta: String,
    },
    #[serde(rename = "response.refusal.done")]
    RefusalDone {
        sequence_number: u64,
        output_index: u32,
        item_id: String,
        content_index: u32,
        refusal: String,
    },
    #[serde(rename = "response.function_call_arguments.delta")]
    ArgsDelta {
        sequence_number: u64,
        output_index: u32,
        item_id: String,
        delta: String,
    },
    #[serde(rename = "response.function_call_arguments.done")]
    ArgsDone {
        sequence_number: u64,
        output_index: u32,
        item_id: String,
        arguments: String,
    },
}

impl Event {
    fn sequence(&self) -> u64 {
        match self {
            Self::Created {
                sequence_number, ..
            }
            | Self::InProgress {
                sequence_number, ..
            }
            | Self::Completed {
                sequence_number, ..
            }
            | Self::Incomplete {
                sequence_number, ..
            }
            | Self::ItemAdded {
                sequence_number, ..
            }
            | Self::ItemDone {
                sequence_number, ..
            }
            | Self::PartAdded {
                sequence_number, ..
            }
            | Self::PartDone {
                sequence_number, ..
            }
            | Self::TextDelta {
                sequence_number, ..
            }
            | Self::TextDone {
                sequence_number, ..
            }
            | Self::RefusalDelta {
                sequence_number, ..
            }
            | Self::RefusalDone {
                sequence_number, ..
            }
            | Self::ArgsDelta {
                sequence_number, ..
            }
            | Self::ArgsDone {
                sequence_number, ..
            } => *sequence_number,
        }
    }
}

#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "message part and function arguments have separate completion phases before item completion"
)]
struct State {
    item: Item,
    part: Option<Part>,
    text_done: bool,
    part_done: bool,
    args_done: bool,
    done: bool,
}

/// Decoder and stateful assembler for the supported Responses event subset.
#[derive(Debug)]
pub struct OpenAiStream {
    decoder: SseDecoder,
    tools: Assembler,
    items: BTreeMap<u32, State>,
    response_id: Option<String>,
    next: u64,
    started: bool,
    terminal: bool,
    pending: Option<OpenAiOutcome>,
    failed: bool,
}

impl OpenAiStream {
    /// Compile the named tool contracts before consuming any provider data.
    ///
    /// # Errors
    /// Invalid or excessive schemas fail before any stream is consumed.
    pub fn new(contracts: BTreeMap<String, Value>) -> Result<Self, StreamError> {
        Ok(Self {
            decoder: SseDecoder::new(),
            tools: Assembler::new(contracts)?,
            items: BTreeMap::new(),
            response_id: None,
            next: 0,
            started: false,
            terminal: false,
            pending: None,
            failed: false,
        })
    }

    /// Consume bytes. Results remain private until `finish` confirms EOF.
    ///
    /// # Errors
    /// Malformed, unsupported, oversized or inconsistent events poison the stream.
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
        if self.terminal && !bytes.is_empty() && !(self.decoder.skip_lf && bytes == b"\n") {
            return Err(StreamError::InvalidEvent);
        }
        for raw in self.decoder.push(bytes)? {
            if self.terminal || self.pending.is_some() {
                return Err(StreamError::InvalidEvent);
            }
            let value: Value =
                serde_json::from_str(&raw.data).map_err(|_| StreamError::InvalidEvent)?;
            if value.get("type").and_then(Value::as_str) != Some(raw.event.as_str()) {
                return Err(StreamError::InvalidEvent);
            }
            let event: Event =
                serde_json::from_str(&raw.data).map_err(|_| StreamError::InvalidEvent)?;
            if event.sequence() != self.next {
                return Err(StreamError::InvalidEvent);
            }
            self.next = self.next.checked_add(1).ok_or(StreamError::LimitExceeded)?;
            self.pending = self.apply(event)?;
        }
        if self.pending.is_some() {
            // Do not release a proposal when the same chunk already contains
            // a partial trailing event that makes the stream incomplete.
            self.decoder.finish()?;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one exhaustive dispatch keeps each supported wire event visible together"
    )]
    fn apply(&mut self, event: Event) -> Result<Option<OpenAiOutcome>, StreamError> {
        match event {
            Event::Created { response, .. } => {
                if self.response_id.is_some()
                    || response.status != "in_progress"
                    || !response.output.is_empty()
                    || response.usage.is_some()
                    || response.incomplete_details.is_some()
                {
                    return Err(StreamError::InvalidEvent);
                }
                Self::basic_response(&response)?;
                self.response_id = Some(response.id);
            }
            Event::InProgress { response, .. } => {
                self.check_response(&response, "in_progress")?;
                if self.started
                    || !response.output.is_empty()
                    || response.usage.is_some()
                    || response.incomplete_details.is_some()
                {
                    return Err(StreamError::InvalidEvent);
                }
                self.started = true;
            }
            Event::Completed { response, .. } => return self.complete(response),
            Event::Incomplete { response, .. } => {
                self.check_response(&response, "incomplete")?;
                if !self.started
                    || !matches!(
                        response
                            .incomplete_details
                            .as_ref()
                            .map(|v| v.reason.as_str()),
                        Some("max_output_tokens")
                    )
                {
                    return Err(StreamError::InvalidEvent);
                }
                self.terminal = true;
                return Ok(Some(OpenAiOutcome::Truncated {
                    usage: response.usage.map(Into::into),
                }));
            }
            Event::ItemAdded {
                output_index, item, ..
            } => self.item_added(output_index, item)?,
            Event::ItemDone {
                output_index, item, ..
            } => self.item_done(output_index, item)?,
            Event::PartAdded {
                output_index,
                item_id,
                content_index,
                part,
                ..
            } => self.part_added(output_index, &item_id, content_index, part)?,
            Event::PartDone {
                output_index,
                item_id,
                content_index,
                part,
                ..
            } => self.part_done(output_index, &item_id, content_index, &part)?,
            Event::TextDelta {
                output_index,
                item_id,
                content_index,
                delta,
                logprobs,
                ..
            } => {
                if !logprobs.is_empty() {
                    return Err(StreamError::InvalidEvent);
                }
                self.part_delta(output_index, &item_id, content_index, &delta, false)?;
            }
            Event::RefusalDelta {
                output_index,
                item_id,
                content_index,
                delta,
                ..
            } => self.part_delta(output_index, &item_id, content_index, &delta, true)?,
            Event::TextDone {
                output_index,
                item_id,
                content_index,
                text,
                logprobs,
                ..
            } => {
                if !logprobs.is_empty() {
                    return Err(StreamError::InvalidEvent);
                }
                self.text_done(output_index, &item_id, content_index, &text, false)?;
            }
            Event::RefusalDone {
                output_index,
                item_id,
                content_index,
                refusal,
                ..
            } => self.text_done(output_index, &item_id, content_index, &refusal, true)?,
            Event::ArgsDelta {
                output_index,
                item_id,
                delta,
                ..
            } => self.args_delta(output_index, &item_id, &delta)?,
            Event::ArgsDone {
                output_index,
                item_id,
                arguments,
                ..
            } => self.args_done(output_index, &item_id, &arguments)?,
        }
        Ok(None)
    }

    fn basic_response(response: &Response) -> Result<(), StreamError> {
        if response.id.is_empty()
            || response.object != "response"
            || !response.created_at.is_finite()
            || response
                .text
                .as_ref()
                .is_some_and(|t| t.format.kind != "text")
            || response
                .tools
                .as_ref()
                .is_some_and(|tools| tools.iter().any(|tool| tool.kind != "function"))
        {
            return Err(StreamError::InvalidEvent);
        }
        Ok(())
    }
    fn check_response(&self, response: &Response, status: &str) -> Result<(), StreamError> {
        Self::basic_response(response)?;
        if self.response_id.as_deref() != Some(response.id.as_str()) || response.status != status {
            return Err(StreamError::InvalidEvent);
        }
        Ok(())
    }
    fn item_added(&mut self, index: u32, item: Item) -> Result<(), StreamError> {
        if !self.started
            || self.items.len() >= MAX_TOOLS
            || u32::try_from(self.items.len()) != Ok(index)
            || self.items.contains_key(&index)
        {
            return Err(StreamError::InvalidEvent);
        }
        let incoming_id = match &item {
            Item::Message { id, .. } | Item::Function { id, .. } => id,
        };
        if self.items.values().any(|state| match &state.item {
            Item::Message { id, .. } | Item::Function { id, .. } => id == incoming_id,
        }) {
            return Err(StreamError::InvalidEvent);
        }
        match &item {
            Item::Message {
                id,
                status,
                role,
                content,
            } if !id.is_empty()
                && status == "in_progress"
                && role == "assistant"
                && content.is_empty() => {}
            Item::Function {
                id,
                call_id,
                name,
                arguments,
                status,
            } if !id.is_empty()
                && !call_id.is_empty()
                && arguments.is_empty()
                && status.as_deref().is_none_or(|s| s == "in_progress") =>
            {
                self.tools.accept(
                    &json!({"type":"tool_start","index":index,"id":call_id,"name":name})
                        .to_string(),
                )?;
            }
            _ => return Err(StreamError::InvalidEvent),
        }
        self.items.insert(
            index,
            State {
                item,
                part: None,
                text_done: false,
                part_done: false,
                args_done: false,
                done: false,
            },
        );
        Ok(())
    }
    fn state(&mut self, index: u32, id: &str) -> Result<&mut State, StreamError> {
        let state = self
            .items
            .get_mut(&index)
            .ok_or(StreamError::InvalidEvent)?;
        let expected = match &state.item {
            Item::Message { id, .. } | Item::Function { id, .. } => id,
        };
        if expected != id || state.done {
            return Err(StreamError::InvalidEvent);
        }
        Ok(state)
    }
    fn part_added(
        &mut self,
        index: u32,
        id: &str,
        content_index: u32,
        part: Part,
    ) -> Result<(), StreamError> {
        let state = self.state(index, id)?;
        if content_index != 0
            || !matches!(state.item, Item::Message { .. })
            || state.part.is_some()
            || !part.content()?.is_empty()
        {
            return Err(StreamError::InvalidEvent);
        }
        state.part = Some(part);
        Ok(())
    }
    fn part_delta(
        &mut self,
        index: u32,
        id: &str,
        content_index: u32,
        delta: &str,
        refusal: bool,
    ) -> Result<(), StreamError> {
        let state = self.state(index, id)?;
        if content_index != 0 || state.text_done {
            return Err(StreamError::InvalidEvent);
        }
        let target = match state.part.as_mut() {
            Some(Part::Text { text, .. }) if !refusal => text,
            Some(Part::Refusal { refusal: text }) if refusal => text,
            _ => return Err(StreamError::InvalidEvent),
        };
        if target.len().saturating_add(delta.len()) > MAX_EVENT {
            return Err(StreamError::LimitExceeded);
        }
        target.push_str(delta);
        Ok(())
    }
    fn text_done(
        &mut self,
        index: u32,
        id: &str,
        content_index: u32,
        value: &str,
        refusal: bool,
    ) -> Result<(), StreamError> {
        let state = self.state(index, id)?;
        let part = state.part.as_ref().ok_or(StreamError::InvalidEvent)?;
        if content_index != 0
            || state.text_done
            || part.content()? != value
            || matches!(part, Part::Refusal { .. }) != refusal
        {
            return Err(StreamError::InvalidEvent);
        }
        state.text_done = true;
        Ok(())
    }
    fn part_done(
        &mut self,
        index: u32,
        id: &str,
        content_index: u32,
        part: &Part,
    ) -> Result<(), StreamError> {
        let state = self.state(index, id)?;
        if content_index != 0
            || !state.text_done
            || state.part_done
            || state.part.as_ref() != Some(part)
        {
            return Err(StreamError::InvalidEvent);
        }
        state.part_done = true;
        Ok(())
    }
    fn args_delta(&mut self, index: u32, id: &str, delta: &str) -> Result<(), StreamError> {
        let state = self.state(index, id)?;
        let Item::Function { arguments, .. } = &mut state.item else {
            return Err(StreamError::InvalidEvent);
        };
        if state.args_done || arguments.len().saturating_add(delta.len()) > crate::MAX_ARGUMENTS {
            return Err(StreamError::LimitExceeded);
        }
        arguments.push_str(delta);
        self.tools
            .accept(&json!({"type":"tool_delta","index":index,"arguments":delta}).to_string())?;
        Ok(())
    }
    fn args_done(&mut self, index: u32, id: &str, args: &str) -> Result<(), StreamError> {
        let state = self.state(index, id)?;
        let Item::Function { arguments, .. } = &state.item else {
            return Err(StreamError::InvalidEvent);
        };
        if state.args_done || arguments != args {
            return Err(StreamError::InvalidEvent);
        }
        state.args_done = true;
        Ok(())
    }
    fn item_done(&mut self, index: u32, item: Item) -> Result<(), StreamError> {
        let state = self
            .items
            .get_mut(&index)
            .ok_or(StreamError::InvalidEvent)?;
        if state.done {
            return Err(StreamError::InvalidEvent);
        }
        match (&state.item, &item) {
            (
                Item::Message { id, .. },
                Item::Message {
                    id: new_id,
                    status,
                    role,
                    content,
                },
            ) if id == new_id
                && status == "completed"
                && role == "assistant"
                && state.part_done
                && content.as_slice() == state.part.as_slice() => {}
            (
                Item::Function {
                    id,
                    call_id,
                    name,
                    arguments,
                    ..
                },
                Item::Function {
                    id: ni,
                    call_id: nc,
                    name: nn,
                    arguments: na,
                    status,
                },
            ) if id == ni
                && call_id == nc
                && name == nn
                && arguments == na
                && state.args_done
                && status.as_deref().is_none_or(|s| s == "completed") =>
            {
                self.tools
                    .accept(&json!({"type":"tool_end","index":index}).to_string())?;
            }
            _ => return Err(StreamError::InvalidEvent),
        }
        state.item = item;
        state.done = true;
        Ok(())
    }
    fn complete(&mut self, response: Response) -> Result<Option<OpenAiOutcome>, StreamError> {
        self.check_response(&response, "completed")?;
        if !self.started
            || response.incomplete_details.is_some()
            || self.items.values().any(|item| !item.done)
            || !response
                .output
                .iter()
                .eq(self.items.values().map(|s| &s.item))
        {
            return Err(StreamError::InvalidEvent);
        }
        let mut text = String::new();
        let mut refusal = None;
        for state in self.items.values() {
            if let Some(part) = &state.part {
                match part {
                    Part::Text { text: value, .. } => {
                        if text.len().saturating_add(value.len()) > MAX_EVENT {
                            return Err(StreamError::LimitExceeded);
                        }
                        text.push_str(value);
                    }
                    Part::Refusal { refusal: value } => {
                        if refusal.replace(value.clone()).is_some() {
                            return Err(StreamError::InvalidEvent);
                        }
                    }
                }
            }
        }
        let usage = response.usage.map(Into::into);
        self.terminal = true;
        if let Some(reason) = refusal {
            return Ok(Some(OpenAiOutcome::Refusal { reason, usage }));
        }
        let result = self.tools.accept(r#"{"type":"complete"}"#)?;
        match result {
            Some(ModelOutcome::Success { tools, .. }) => {
                Ok(Some(OpenAiOutcome::Success { text, tools, usage }))
            }
            Some(ModelOutcome::InvalidSchema) => Ok(Some(OpenAiOutcome::InvalidSchema { usage })),
            _ => Err(StreamError::InvalidEvent),
        }
    }

    /// Declare transport EOF, confirm framing, and take the terminal result once.
    /// Call only after the transport has actually reached EOF.
    ///
    /// # Errors
    /// Incomplete or previously rejected streams cannot succeed.
    pub fn finish(&mut self) -> Result<Option<OpenAiOutcome>, StreamError> {
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
