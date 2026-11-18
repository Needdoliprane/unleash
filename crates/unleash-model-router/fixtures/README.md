# OpenAI Responses SSE fixtures, 2026-11-18

Synthetic, deterministic fixtures modeled on the official
[Responses streaming reference](https://developers.openai.com/api/reference/resources/responses/streaming-events)
and [function calling guide](https://developers.openai.com/api/docs/guides/function-calling#streaming),
consulted 2026-11-18. They are not captured provider responses or evidence of a
live qualification. Identifiers, text, counters and timestamps are synthetic.

The adapter accepts only the implemented closed subset: lifecycle events,
message text or refusal with a single content part, function calls, empty
annotations/logprobs and the represented response-object fields. It rejects
other event types, fields, nested variants, content parts and incomplete reasons
explicitly. No hosted tools, transport, network, credentials or model calls are
exercised. A real adapter requires documented version pinning, representative
provider fixtures, and qualification before use.

The `anthropic-messages-2026-11-18-*` fixtures are synthetic and based on the
official [Messages streaming guide](https://platform.claude.com/docs/en/build-with-claude/streaming)
and [stop-reason reference](https://platform.claude.com/docs/en/build-with-claude/handling-stop-reasons),
consulted 2026-11-18. They cover text and client-side `tool_use` only, basic
usage counters, `ping`, and documented terminal reasons. Hosted tools, thinking,
citations, pause/continuation, rich usage variants and error events are rejected.
They do not demonstrate live interoperability or HTTP behavior.
