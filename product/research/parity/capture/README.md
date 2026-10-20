# The capture of the primary reference

The tooling for the phase 7 capture of Claude Code 2.1.281 through the Python Agent SDK 0.2.159, as the design's section g fixes it. The owner runs it with her key; nothing here holds a key, a capture, or a path of her machine, and the capture itself stays out of the repository and out of any agent's scratchpad (`../README.md`, "How parity is checked").

## What's here

| File               | What it is                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `capture.py`       | The pinned script of `../claude-code.md`, "Reference configuration", with `TS = False` so every tool definition goes up front, the in-process SDK MCP server `lab` in place of the server under test, the body log on, the OTel logs exporter pointed at the sink, and the prompt of each run. `--run R0..R7 --model <id> --out <dir>`, `--base-url` for R5, `--cwd` (default `/private/tmp/lab-capture/cwd`), `--config-dir`, `--effort`, `--with-attribution`. A run's bodies land under `<out>/<run>/<model>/` (R4 under `.../text/` and `.../write/`), with the SDK's messages in `messages.txt` beside them. |
| `otlp_sink.py`     | A listener on `127.0.0.1:4318` that answers every POST with 200 and keeps nothing. Claude Code writes the body log only while a logs exporter is on, and the exporter needs somewhere to send. No Docker needed.                                                                                                                                                                                                                                                                                                                                                                                                  |
| `mock_upstream.py` | The upstream for R5. Plays one status sequence per invocation, `--sequence 500x3-then-200`, logs the time, status, path and the two retry headers it sent, never a request header or body, and streams its 200 as server-sent events since the reference streams. Listens on `127.0.0.1:8765`.                                                                                                                                                                                                                                                                                                                    |
| `redact.py`        | The handover step: refuses a directory holding any `sk-ant-` string, drops every top-level `metadata`, replaces the working directory's path with `<CWD>`. In place.                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `skeleton.py`      | Reduces request bodies to their skeletons: top-level parameters, `metadata` as its keys alone, block types and order, every `cache_control` position, tool names masked by position, text replaced by its byte length. A directory stands for the request files in it and not in its subdirectories; a file that is no request body is refused by its keys. For the field-by-field diff of lablet's requests against the reference's.                                                                                                                                                                             |
| `prompt.md`        | The system prompt: neutral instructions of over 1,200 words, so the prefix clears the cacheable minimum, 1,024 tokens on Sonnet 5 and 512 on the 5.5 models; below it R3 proves nothing. There's no tokenizer here, so the length is held in words.                                                                                                                                                                                                                                                                                                                                                               |
| `task.md`          | The R2 and R3 task: file work alone, printing nothing of the host. A write, reads with `offset` and `limit`, a `wc`, writes, and a text ending, in ten steps.                                                                                                                                                                                                                                                                                                                                                                                                                                                     |

The stand-in server `lab` offers `bash`, `read_file` and `write_file` under lablet's own descriptions and input schemas as the adapter sends them, timeout suffix included, copied from `lablet/crates/adapters/secondary/tools-builtin/src/{bash,read_file,write_file}.rs` and `executor.rs`; each runs over the working directory as lablet's built-ins run over their root, and returns `content` and `is_error` as lablet's do. The reference prefixes the names, `mcp__lab__bash`; `skeleton.py` takes the prefix off before masking, so both sides land on the same mask.

## Before the capture

Every `.py` here is read before it's run, and run with `python3 -I`, which ignores the environment's Python variables and the current directory on the import path.

The one check the design asks for before the capture: that the SDK's `@tool` takes a full JSON schema with per-property descriptions, so the `tools` comparison is exact. Read at 0.2.159: `tool(name, description, input_schema)` takes "a JSON Schema dictionary for full validation", and `_build_input_schema` sends a dict that has a string `type` and a `properties` key on the wire as it is (`claude_agent_sdk/__init__.py`). So `lab`'s schemas go up with `description`, `minimum`, `required` and `additionalProperties: false` exactly as lablet's do, and the comparison of `tools` is exact apart from the name prefix. The SDK validates arguments against that schema before the handler runs and reports a mismatch as an error result, where lablet's built-ins report `the arguments of <tool> don't fit`.

The SDK needs Python 3.10 or later. macOS's own `python3` is 3.9, so the venv below is made with a newer interpreter, such as Homebrew's or `uv`'s.

## The commands

In one sitting of two to three hours and a few dollars. `<repo>` is the checkout; everything else is under `/private/tmp/lab-capture`, a path with no user name in it, which is why the cwd lives there.

```bash
# once
python3 -m venv /private/tmp/lab-capture/venv && . /private/tmp/lab-capture/venv/bin/activate
pip install claude-agent-sdk==0.2.159
export ANTHROPIC_API_KEY=...                       # her shell, never a file or a flag
mkdir -p /private/tmp/lab-capture/bodies /private/tmp/lab-capture/cwd /private/tmp/lab-capture/config
cd /private/tmp/lab-capture
python3 -I <repo>/product/research/parity/capture/otlp_sink.py &   # the listener on 127.0.0.1:4318

# R0 smoke: one prompt; then confirm the files and that no key reached them
python3 -I <repo>/product/research/parity/capture/capture.py --run R0 --model claude-opus-5-5 --out bodies
ls bodies/R0/*/*.request.json && grep -rl sk-ant- bodies || echo "no key in the bodies"

# R1 baseline, one request per model
for m in claude-sonnet-5 claude-sonnet-5-5 claude-opus-5-5 claude-haiku-5-5 claude-fable-5-1; do
  python3 -I <repo>/product/research/parity/capture/capture.py --run R1 --model $m --out bodies
done
# R2 ten turns on the stand-in, on the starter's model, from an empty cwd as the replay's root is
rm -rf /private/tmp/lab-capture/cwd
python3 -I <repo>/product/research/parity/capture/capture.py --run R2 --model claude-opus-5-5 --out bodies
# R3 R2 again within five minutes, same files, before anything else runs
python3 -I <repo>/product/research/parity/capture/capture.py --run R3 --model claude-opus-5-5 --out bodies
# R2 on the previous generation, from an empty cwd again
rm -rf /private/tmp/lab-capture/cwd
python3 -I <repo>/product/research/parity/capture/capture.py --run R2 --model claude-sonnet-5 --out bodies
# R4 the output cap, a long text and a 2,000-word write_file
CLAUDE_CODE_MAX_OUTPUT_TOKENS=300 python3 -I <repo>/product/research/parity/capture/capture.py --run R4 --model claude-opus-5-5 --out bodies
# R5 the mock upstream, one sequence at a time, a dummy key in the script's env so the real one never reaches it
python3 -I <repo>/product/research/parity/capture/mock_upstream.py --sequence 529x12 &
python3 -I <repo>/product/research/parity/capture/capture.py --run R5 --model claude-opus-5-5 --base-url http://127.0.0.1:8765 --out bodies
# ... the sequences: 529x12 (on claude-opus-5-5 and claude-sonnet-5), 500x3-then-200, 429-retry-after-7-then-200,
#     429-retry-after-61, 500-x-should-retry-false, 400-prompt-too-long-then-200
# R6 forty-odd calls returning about 20 KB each, compaction off
python3 -I <repo>/product/research/parity/capture/capture.py --run R6 --model claude-opus-5-5 --out bodies
# R7 "Reply with nothing."
python3 -I <repo>/product/research/parity/capture/capture.py --run R7 --model claude-opus-5-5 --out bodies

# the handover
python3 -I <repo>/product/research/parity/capture/redact.py bodies
```

Notes on the commands:

- The model is `claude-opus-5-5` where the design wrote `claude-sonnet-5-5`: the owner's choice of the starter's model, 2026-10-20. The design's second R2 run was on Opus 5.5 beside the starter's model, for the generation contrast in the reminder channel (`role: "system"` on Opus 5.5 against `<system-reminder>` text on Sonnet 5); with Opus 5.5 the starter's model, the second R2 runs on `claude-sonnet-5` for the same contrast. R3 comes directly after the first R2, since the cache entry it reads lives five minutes from the last request that touched it, and a ten-turn run on another model between them could outlast that.
- The working directory is shared by every run, and each R2 starts from it empty: `capture.py` makes it again, and the replay runs lablet on an empty root, so a `ls` in the task sees the same files on both sides. R3 keeps R2's files, as the design asks.
- `--out bodies` gives each run its own directory, `bodies/<run>/<model>/`, so R1's five models and R5's sequences don't mix in one body log; the design's `ls bodies/*.request.json` is `ls bodies/R0/*/*.request.json`. R5 bodies land in the same directory for every sequence of a model: run each sequence with the mock restarted, and move or rename `bodies/R5/<model>/` between sequences if the files are to be told apart; the mock's own log on standard output names the sequence.
- R5 is one invocation of `mock_upstream.py` per sequence. Stop the mock (`kill %2`, or `fg` and Ctrl-C) and start it with the next `--sequence` before the next `capture.py`. Fable 5.1 returns a 400 on an organisation whose retention is under 30 days: a fact to record, not a bug.
- `capture.py` states no `effort`, so each model's own default goes up, which is what R1 reads. The pinned script's `effort="high"` is `--effort high`.
- R1 with `--with-attribution` leaves `CLAUDE_CODE_ATTRIBUTION_HEADER` unset, for the "with and without `=0`" reading; the design's R1 line runs with the pin, so run it once more with the flag if the reading is wanted.
- R4's two parts, the long text and the 2,000-word `write_file`, are two sessions of the one invocation, under `bodies/R4/<model>/text/` and `.../write/`.
- The reference's side requests, if any (a body whose `query_source` isn't the main conversation's), are in the body log too. The replay below serves the files of one conversation in order, so copy the main conversation's files into a directory of their own when there are others.

The handover: the capture stays where it was made, out of the repository and out of the scratchpad, and is read in place after `redact.py` has run. `redact.py` exits 2 and changes nothing when any file holds `sk-ant-`; a clean run prints how many files it read, how many `metadata` keys it dropped and how many paths it replaced.

## The comparison

Two steps, by hand, at the capture. First the replay: `lablet/crates/adapters/secondary/provider-anthropic/tests/it/replay.rs`, `#[ignore]`d, serves each recorded response in order from wiremock, runs lablet with the three built-ins on an empty scratch root with the capture's `prompt.md` and `task.md`, and writes what lablet sent under `<capture>/lablet-replay/`:

```bash
LABLET_REPLAY_CAPTURE=/private/tmp/lab-capture/bodies/R2/claude-opus-5-5 \
  cargo test --locked --manifest-path <repo>/lablet/Cargo.toml -p lablet-provider-anthropic --test it -- --ignored replay
```

Then the skeletons, both sides, and a diff on the rows of the design's table:

```bash
python3 -I <repo>/product/research/parity/capture/skeleton.py bodies/R2/claude-opus-5-5 --out skeletons/reference
python3 -I <repo>/product/research/parity/capture/skeleton.py bodies/R2/claude-opus-5-5/lablet-replay --out skeletons/lablet
diff -r skeletons/reference skeletons/lablet
```

For E1 and the key order of a replayed `tool_use.input`, the skill's `prefix_diff.py` (`claude-api/shared/preserved-thinking-migration/`) diffs adjacent reference requests as the API does, `cache_control` stripped.

## The optional probe

The design's d.3 names one stronger test of the thinking-block check, hers alone, for an organisation where the check isn't enforced by default: one request of a body lablet recorded, with `prefix_mismatch_behavior: "error"` under the `thinking-binding-controls-2026-08-01` beta, which opts that request into enforcement whatever the account's age. A 200 confirms lablet's sorted replay; a 400 whose text says the block is "bound to a different conversation" refutes it. With `body.json` a recorded lablet request whose `thinking` object has `"block_binding": {"prefix_mismatch_behavior": "error"}` added, and `max_tokens` cut to 16 since the verdict comes before the first output token:

```bash
curl -sS https://api.anthropic.com/v1/messages -H "x-api-key: $ANTHROPIC_API_KEY" -H "anthropic-version: 2023-06-01" -H "anthropic-beta: thinking-binding-controls-2026-08-01" -H "content-type: application/json" --data @body.json
```

Lablet sends no beta header itself; this is the owner's request, from her shell, and nothing of it enters the tree.
