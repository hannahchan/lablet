//! Secondary adapter: a `ModelProvider` that plays a script, so a run needs
//! no model and no key. It's a product feature, for testing what's built
//! around lablet, and lablet's own examples and smoke tests run on it.
//!
//! # The script
//!
//! A script is a list of entries, in YAML or JSON. An entry answers one
//! attempt of a provider call, and entries are played in the order they're
//! written, each once: the first attempt of a run gets the first entry, and
//! an attempt that fails is followed by the entry after it, which answers
//! the retry when the loop makes one. The provider reads nothing of a
//! request but its deadline, so a script says what the model answers
//! whatever it was asked.
//!
//! An entry is a `response` or an `error`.
//!
//! ## A response
//!
//! ```yaml
//! - response:
//!     content:
//!       - text: I'll list the files.
//!       - tool_use:
//!           id: call_1
//!           name: bash
//!           input:
//!             json: { command: ls }
//!     finish: tool_use
//!     usage: { input_tokens: 1200, output_tokens: 80 }
//!     response_id: msg_01
//!     response_model: scripted-2026-09
//!     latency: 250ms
//! ```
//!
//! | Key              | Needed | Holds                                                                  |
//! | ---------------- | ------ | ---------------------------------------------------------------------- |
//! | `content`        | yes    | The blocks of the response, in order. `[]` is one that says nothing.   |
//! | `finish`         | yes    | Why the model stopped.                                                 |
//! | `usage`          | no     | The tokens the call used.                                              |
//! | `response_id`    | no     | The provider's id for the response.                                    |
//! | `response_model` | no     | The model that answered.                                               |
//! | `latency`        | no     | How long the attempt takes, in real time. Left out, it takes none.     |
//!
//! `finish` is `end_turn`, `tool_use`, `max_tokens`, `context_window` or
//! `refusal`, or either provider API's own spelling of one of them, as
//! `stop`, `tool_calls` and `length` are. Any other text is a reason lablet
//! has no name for, and is kept as it's written.
//!
//! `usage` holds `input_tokens`, `output_tokens`,
//! `reasoning_output_tokens`, `cache_read_tokens` and `cache_write_tokens`.
//! The input count includes the cached tokens and the output count the
//! reasoning tokens. Any of them may be left out. The first two are then
//! zero. Each of the other three is then a count the provider didn't
//! report, which isn't a count of zero: the run's outcome writes it as
//! `null`, and its telemetry leaves the attribute out.
//!
//! A duration is text in the syntax the config's durations have: `250ms`,
//! `2s`, `1m 30s`.
//!
//! ## The blocks of a response
//!
//! ```yaml
//! - response:
//!     content:
//!       - thinking: { text: The user wants the files listed., signature: c2lnbmVk }
//!       - thinking: { text: A provider that signs nothing leaves the signature out. }
//!       - redacted_thinking: { data: ZW5jcnlwdGVk }
//!       - text: I'll list the files and read the first.
//!       - tool_use: { id: call_1, name: bash, input: { json: { command: ls } } }
//!       - tool_use: { id: call_2, name: read_file, input: { unparsed: '{"path": "READ' } }
//!       - opaque: { provider: openai, payload: { type: reasoning, encrypted_content: abc } }
//!     finish: tool_use
//! ```
//!
//! A tool call's `input` is `json`, the arguments as they parsed, or
//! `unparsed`, the text of arguments that didn't, which the loop answers
//! without running a tool. An `opaque` block's `provider` is `anthropic`,
//! `openai` or `fake`.
//!
//! ## An error
//!
//! ```yaml
//! - error:
//!     kind: retryable
//!     message: 529 overloaded
//!     usage: { input_tokens: 1200 }
//!     retry_after: 2s
//!     latency: 40ms
//! ```
//!
//! | Key           | Needed | Holds                                                                  |
//! | ------------- | ------ | ---------------------------------------------------------------------- |
//! | `kind`        | yes    | `retryable`, `context_exhausted`, `auth`, `fatal` or `malformed`.      |
//! | `message`     | no     | What the provider said. Left out, it names the entry and the kind.     |
//! | `usage`       | no     | The tokens the failed attempt reported. Left out, it reported none.    |
//! | `retry_after` | no     | The wait the server asked for before the next attempt.                 |
//! | `latency`     | no     | How long the attempt takes before it fails. Left out, it takes none.   |
//!
//! A message is cut to [`lablet_run::ERROR_MESSAGE_MAX_BYTES`], as every
//! provider's is.
//!
//! ## The same in JSON
//!
//! ```json
//! [
//!   { "response": {
//!       "content": [{ "text": "Done." }],
//!       "finish": "end_turn",
//!       "usage": { "input_tokens": 1200, "output_tokens": 80 },
//!       "latency": "250ms"
//!   } },
//!   { "error": { "kind": "fatal", "message": "unknown model" } }
//! ]
//! ```
//!
//! # What's refused
//!
//! A person writes a script, so [`Script::read`] refuses what a slip of
//! theirs would look like, and names the script, the entry and the block,
//! each counted from 1:
//!
//! ```text
//! script "scripts/smoke.yaml": entry 3, block 2: unknown field `nme`, expected one of `id`, `name`, `input`
//! ```
//!
//! - A key the format doesn't have, wherever it is. `input_token: 12` is
//!   an error and never a count of zero.
//! - A key that's needed and missing, and a key written twice.
//! - A value of the wrong kind. Both formats are read the same way, so
//!   text that reads as a number, as `id: 1` does, is a number: quote it.
//! - What the domain refuses: a tool name no provider accepts, a tool call
//!   id that's empty, and a tool call id on two blocks of one response.
//! - A script with no entries.
//!
//! So a script that was read is played as it's written, and nothing in it
//! is refused during a run.
//!
//! # Deadlines
//!
//! An attempt is given a deadline, and the provider holds it in real time.
//! When an entry's latency is as long as the deadline or longer, the
//! attempt waits until the deadline and then fails with the kind
//! `retryable`, which is how every provider reports a deadline it
//! reached. The entry has been played all the same, and what it held is
//! never answered, its usage included.
//!
//! # Dropped attempts
//!
//! An attempt that's dropped before it answers, as the loop drops the one
//! in flight when a run is cancelled, stops where it is: its wait ends with
//! it, and nothing of it goes on. Its entry has been played, so the attempt
//! after it is answered by the entry after that.
//!
//! # When the script runs out
//!
//! An attempt made after the last entry fails at once with the kind
//! `fatal`, so the run ends with `provider_error`, and its `error` names
//! the script and says to add an entry or to have the last response end
//! the run. Another attempt would change nothing, which is what `fatal`
//! says. A kind the loop retries would have it wait out every backoff and
//! then report `retries_exhausted`, as if a provider had been unreliable
//! and not a script short.
//!
//! # More than one run
//!
//! The port tells a provider nothing of runs, so the provider keeps its
//! place from one call to the next, whichever run made it.
//! [`FakeProvider::rewind`] puts the script back at its first entry, for
//! whoever starts a run to give each run the script from its start.
//!
//! # Reading one
//!
//! ```
//! use lablet_provider_fake::{FakeProvider, Script, ScriptFormat, ScriptSource};
//! use lablet_run::ModelProvider as _;
//!
//! let script = Script::read(ScriptSource {
//!     name: "scripts/smoke.yaml",
//!     text: "
//! - error: { kind: retryable, message: 529 overloaded, retry_after: 2s }
//! - response: { content: [{ text: Done. }], finish: end_turn }
//! ",
//!     format: ScriptFormat::Yaml,
//! })?;
//! assert_eq!(script.entries(), 2);
//!
//! let provider = FakeProvider::new("scripted-1", script);
//! assert_eq!(provider.model().name, "scripted-1");
//! # Ok::<(), lablet_provider_fake::ScriptError>(())
//! ```

mod provider;
mod script;
mod tree;

pub use provider::FakeProvider;
pub use script::{Script, ScriptError, ScriptFault, ScriptFormat, ScriptSource};

#[cfg(test)]
mod tests;
