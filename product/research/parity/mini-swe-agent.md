# mini-swe-agent

The minimal baseline reference, pinned at v2.4.6 (2026-07-23). Researched 2026-09-24 from the source at that tag, its release notes and issues, and the SWE-bench leaderboards. The "Fit for lablet" section compares against choices under consideration on 2026-09-24, several of them not yet recorded in `decisions.md`. The [matrix](matrix.md) has lablet's current behaviour and the proposals.

Paths are `src/minisweagent/…` at [tag v2.4.6](https://github.com/SWE-agent/mini-swe-agent/tree/v2.4.6). `#n` means an issue or PR in that repo.

## 1. Identity

- **What it is:** a minimal coding agent. It is a ~100-line `DefaultAgent` with one bash tool and a history that only ever grows (README).
- **Maintainers:** the Princeton/Stanford SWE-bench and SWE-agent team; the PyPI authors are Kilian Lieret and Carlos E. Jimenez.
- **Licence:** MIT.
- **Latest release:** v2.4.6, released 2026-07-23 on GitHub and PyPI. `main` has only 3 small commits since.

## 2. Relevance

- **Leaderboard:** mini is what the SWE-bench "Bash Only" leaderboard runs. All 47 Verified entries are mini runs, and every model gets the same prompt. The top entry is Claude 4.5 Opus (high) at 76.8% on mini 2.0.0 (2026-02-17). The newest entry is dated 2026-02-26 (swebench.com embedded data; [Willison](https://simonwillison.net/2026/Feb/19/swe-bench/)).
- **Research baselines:** Epoch AI bases its SWE-bench prompt on the bash-only runs ([Epoch](https://epoch.ai/benchmarks/swe-bench-verified)). Live-SWE-agent starts from mini ([arXiv 2511.13646](https://arxiv.org/abs/2511.13646)). The README's list of industry users is unverified.
- **MCP:** it can't host MCP tools. The maintainer's advice is to put a CLI in the environment and tell the model about it in the prompt (#563, #470). An MCP question opened 2026-08-07 has no answer (#927).
- **For MCP or skills optimisation:** weak as a harness. It is strong as a "model plus shell only" control.

## 3. Providers

- **Default:** LiteLLM (`models/__init__.py:110-113`). So Anthropic goes through LiteLLM's translation rather than a native Messages client, and OpenAI-compatible servers also work.
- **Other model classes:** Responses API, OpenRouter, Portkey and Requesty.

## 4. Checkability

- **Headless:** `mini -y --exit-immediately -t … -c <cfg> -o run.traj.json`. Set `MSWEA_CONFIGURED=1`, or it stops at a first-run setup prompt (`run/utilities/config.py:62-66`). Hitting a limit with no terminal attached exits cleanly (`agents/interactive.py:80-87`).
- **Custom prompts:** four Jinja templates (system, instance, observation, format error). They come from stackable `-c` files and `key=value` overrides (docs/advanced/v2_migration.md).
- **Restricting tools:** not possible. `tools=[BASH_TOOL]` is hard-coded (`models/litellm_model.py:66-71`), and any other tool name is a format error (`models/utils/actions_toolcall.py:61-62`).
- **Recording proxy:** `model.model_kwargs.api_base` is passed straight to LiteLLM (docs/models/local_models.md). A model LiteLLM has no price for aborts the run unless `cost_tracking: ignore_errors` is set (`models/litellm_model.py:108-126`).
- **Pinning:** litellm isn't pinned (`>=1.75.5`), so pin it too. The text-mode configs don't set `model_class`, so pass `--model-class litellm_textbased` explicitly (`config/mini_textbased.yaml`).

## 5. Loop behaviours

- **a. Action mechanism:** native tool calling with one `bash(command)` function, the default since v2.0.0 (2026-02-11). Up to v1.x it parsed a single fenced block with a regex (v1.17.5 `agents/default.py:36,116-119`). That mode survives as `litellm_textbased`. The README and FAQ still say mini doesn't use tool calling; that is out of date.
- **b. Actions per turn:** several. `swebench.yaml` sets `parallel_tool_calls: true`, and the calls run one after another in call order (`agents/default.py:156`). Text mode allows exactly one (`models/utils/actions_text.py:24-27`).
- **c. Observation formatting and truncation:**
  - stdout and stderr are merged (`environments/local.py:83`).
  - `swebench.yaml` wraps output in `<returncode>`/`<output>`.
  - At 10,000 characters or more it keeps the first 5,000 and the last 5,000 and adds a warning. `mini.yaml` does the same in JSON.
- **d. Format errors and command failures:**
  - A malformed, unknown or missing tool call raises `FormatError`. Mini drops that response from history and sends a user-role error instead (#897).
  - Three format errors in a row end the run as `RepeatedFormatError` (v2.4.0 onward; `agents/default.py:100-114`).
  - Failed or timed-out commands are ordinary observations, with no cap.
- **e. Stop conditions** (`agents/default.py:26-33,132-147`):
  - the submit command (see n);
  - `step_limit`: off by default, 250 in `swebench.yaml`, and format errors count as steps;
  - `cost_limit`: $3 by default;
  - `wall_time_limit_seconds`: added in v2.3.0, checked only before each model call;
  - global cost and call caps set through environment variables;
  - per command: a 30 s timeout (60 s in `swebench.yaml`).
- **f. Max output tokens and cut-offs:**
  - Mini sets no `max_tokens`. For Anthropic, LiteLLM fills in the model's registered maximum, falling back to 4096 (litellm `llms/anthropic/chat/transformation.py`).
  - A response cut off at the limit gets a format error asking for a shorter reply, capped as in (d).
  - Tool calls that still parse from a cut-off response are executed (`models/litellm_model.py:128-135`).
- **g. Retries:** 10 attempts with exponential backoff from 4 to 60 s. Everything is retried except auth, permission, not-found, unsupported-parameter and context-window errors (`models/utils/retry.py`; `litellm_model.py:50-57`).
- **h. Prompt caching:** only for model names that look like Anthropic ones. One cache breakpoint moves to the last message on each call; the system prompt and tools aren't marked (`models/__init__.py:55-60`; `models/utils/cache_control.py`).
- **i. Context management:** none, and history never shrinks. A context overflow ends the run without a retry (`agents/default.py:117-119`).
- **j. Reasoning replay:** each provider message is replayed whole, minus mini's own `extra` field (`litellm_model.py:76-79,99`). LiteLLM rebuilds Anthropic `thinking_blocks`, and the Responses API class replays output items (`models/litellm_response_model.py:29-38`). Whether reasoning is replayed over Chat Completions is unverified.
- **k. Shell semantics:** a new process for every action. Locally that's `Popen(shell=True)`, which means `/bin/sh`, not bash. In Docker it's `docker exec … bash -lc`. Files persist between actions; the working directory and environment variables don't (`environments/local.py:72-92`; `environments/docker.py:101-124`).
- **l. MCP:** none.
- **m. Skills:** none. Asked about loading AGENTS.md, the maintainer said "just tell it in the prompt to check & read it" (#841).
- **n. Completion protocol:** the run ends when a command's first output line is `COMPLETE_TASK_AND_SUBMIT_FINAL_OUTPUT` and it exits 0. The rest of that output is the submission (`environments/local.py:45-56`). A reply with text and no tool call is a format error, so a run can't end on its own.
- **o. Telemetry and trajectory format:** no OpenTelemetry and no hooks. An observer PR was declined in favour of subclassing (#852, #853). A JSON trajectory is saved after every step, with cost, call count, exit status and the raw responses, including token usage (`agents/default.py:159-190`).

**SWE-agent, the parent:**

- Latest release v1.1.0 (2025-05-22); its own README now recommends mini instead.
- Several tool bundles over function calling.
- A stateful shell session kept alive between commands (SWE-ReX; `sweagent/environment/swe_env.py`).
- `last_n_observations` replaces older command outputs with a placeholder (`sweagent/agent/history_processors.py`).
- Up to 3 re-queries after format errors, and caps on timeouts (`sweagent/tools/tools.py`).
- It declined MCP (SWE-agent #861).

## 6. Fit for lablet (opinion)

- **Tool-error cap off by default:** mini supports this for command failures, which it never caps. It contradicts it for malformed or unknown tool calls, which it caps at 3.
- **Truncation at max_tokens is terminal, 32k default:** contradicts. Mini re-prompts, runs tool calls that still parse from a cut-off response, and takes LiteLLM's default.
- **Skills loaded progressively:** neutral, because mini has no skills. The maintainer's "tell the model to read files" advice amounts to progressive loading through bash.
- **MCP servers live across runs:** not applicable.
- **Prompt cache shared across runs:** neutral in intent, but the breakpoints are in different places. Lablet marks the system prompt and tools, so only the fixed prefix is cached. Mini's moving breakpoint caches the growing history. Cached-input token counts will differ sharply.
- **Compaction by masking old tool outputs first, summarisation later:** mini does neither. SWE-agent's `last_n_observations`, which updates in steps to stay cache-friendly, is prior art for masking first. Neither agent summarises.

**Obstacles to parity.** Mini v2's switch to native tool calling removes the biggest gap. Lablet's built-in `bash` is `Exclusive`, so it also runs calls one at a time in call order. What remains:

1. Mini only offers `bash`. Exposing an MCP tool surface means subclassing it, and then it is no longer the reference.
2. Mini completes with a submit command. Lablet completes either when the model stops calling tools or through `task_complete`.
3. Mini drops a malformed turn and re-prompts. Lablet keeps the turn and returns `malformed_input`. Transcripts and token counts will differ.
4. Handling of cut-off responses and the default `max_tokens` differ.
5. Output shaping differs:
   - mini merges stdout and stderr and wraps the output;
   - mini cuts at 10,000 characters and keeps both ends, while lablet cuts at 100,000 bytes and keeps only the start;
   - the `bash` tool schema would also have to match exactly.
6. Mini's limits are in dollars, it retries other 400 errors, and its cache breakpoint sits elsewhere.
7. Token counts have to be recovered from trajectories or from a recording proxy.

## 7. Verdict (opinion)

mini-swe-agent is unsuitable as a primary reference for MCP tool surfaces, because the only tool it can offer is `bash`. It is the best-understood "minimal" baseline profile (the leaderboard-standard prompt, a ~100-line loop, a stateless shell), and a good secondary reference for model and system-prompt comparisons once lablet's bash-only profile can imitate its submit command and head-plus-tail output cut.

Sources: [mini-swe-agent v2.4.6](https://github.com/SWE-agent/mini-swe-agent/tree/v2.4.6), [releases](https://github.com/SWE-agent/mini-swe-agent/releases), [SWE-bench leaderboards](https://www.swebench.com/), [Simon Willison](https://simonwillison.net/2026/Feb/19/swe-bench/), [Epoch AI](https://epoch.ai/benchmarks/swe-bench-verified), [arXiv 2511.13646](https://arxiv.org/abs/2511.13646), [SWE-agent v1.1.0](https://github.com/SWE-agent/SWE-agent/tree/v1.1.0), [LiteLLM](https://github.com/BerriAI/litellm)

## Corrections after verification

A second pass on 2026-09-25 checked this profile against the source at v2.4.6. The matrix already reflects these corrections.

- **Shell (5k).** Locally, a command runs through `/bin/sh` (`Popen(shell=True)`, `environments/local.py`). `bash -lc` is only the Docker environment's default (`environments/docker.py`), and the SWE-bench configuration runs `bash -c`. The `<returncode>` and `<output>` wrapping comes from `swebench.yaml` and `default.yaml`. `mini.yaml` uses JSON instead.
- **Rate-limit hints.** Its own retry, tenacity with exponential waits from 4 s to 60 s, ignores `Retry-After`. Whether LiteLLM honours it underneath is unverified.
- **Refusals.** A refusal without a tool call is a format error, like any reply without a tool call.
- **Sampling.** The shipped configurations set no sampling parameters. Only `drop_params` and `parallel_tool_calls` are set, and `model_kwargs` can add others.
