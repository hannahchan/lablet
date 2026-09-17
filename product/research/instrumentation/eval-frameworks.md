# Evaluation frameworks and benchmarks

Researched: 2026-09-18. Sources are linked inline. Everything here is an inventory of what the source emits or expects; recommendations for lablet are confined to the final section and marked as opinion.

## Sources covered

- **Inspect AI** (UK AISI) v0.3.265, commit 615e284 (2026-09-17). [`log/_log.py`](https://github.com/UKGovernmentBEIS/inspect_ai/blob/main/src/inspect_ai/log/_log.py), [`event/*.py`](https://github.com/UKGovernmentBEIS/inspect_ai/tree/main/src/inspect_ai/event), [`model/_model_output.py`](https://github.com/UKGovernmentBEIS/inspect_ai/blob/main/src/inspect_ai/model/_model_output.py), [Eval Logs docs](https://inspect.aisi.org.uk/eval-logs.html), [Setting Limits docs](https://inspect.aisi.org.uk/setting-limits.html).
- **Harbor** (Laude Institute; Terminal-Bench 2.x runs on it) v0.23.0, commit b07f3bf (2026-09-17). [`models/trial/result.py`](https://github.com/harbor-framework/harbor/blob/main/src/harbor/models/trial/result.py), [`models/agent/context.py`](https://github.com/harbor-framework/harbor/blob/main/src/harbor/models/agent/context.py), [`models/trajectories/*.py`](https://github.com/harbor-framework/harbor/tree/main/src/harbor/models/trajectories), [RFC 0001 ATIF v1.8](https://github.com/harbor-framework/harbor/blob/main/rfcs/0001-trajectory-format.md) (April 2026). [Terminal-Bench paper arXiv:2601.11868](https://arxiv.org/abs/2601.11868) (Jan 2026).
- **SWE-agent** v1.1.0, commit 3ea751c (2026-07-16). [`sweagent/types.py`](https://github.com/SWE-agent/SWE-agent/blob/main/sweagent/types.py), [`agent/models.py`](https://github.com/SWE-agent/SWE-agent/blob/main/sweagent/agent/models.py), [Output files docs](https://swe-agent.com/latest/usage/trajectories/). **mini-SWE-agent** v2.x, commit 04d809c (2026-09-03), [Output files docs](https://mini-swe-agent.com/latest/usage/output_files/).
- **SWE-bench harness** v5.0.2, commit 02e7a74 (2026-09-02). [`harness/grading.py`](https://github.com/SWE-bench/SWE-bench/blob/main/swebench/harness/grading.py), [`harness/reporting.py`](https://github.com/SWE-bench/SWE-bench/blob/main/swebench/harness/reporting.py), [`harness/constants/__init__.py`](https://github.com/SWE-bench/SWE-bench/blob/main/swebench/harness/constants/__init__.py).
- **OpenAI Evals** v3.0.1.post1, commit 8eac7a7 (2026-04-14, effectively archived). [`evals/record.py`](https://github.com/openai/evals/blob/main/evals/record.py), [`evals/base.py`](https://github.com/openai/evals/blob/main/evals/base.py).
- **METR Vivaria** commit 20a6c29 (2026-02-15) and **METR Task Standard** 0.5.0, commit 03236e9 (2025-02-03). [`shared/src/types.ts`](https://github.com/METR/vivaria/blob/main/shared/src/types.ts), [pyhooks reference](https://vivaria.metr.org/reference/pyhooks/), [STANDARD.md](https://github.com/METR/task-standard/blob/main/STANDARD.md).
- **tau2-bench** (Sierra) v1.0.1, commit b7ea907 (2026-09-17). [`data_model/simulation.py`](https://github.com/sierra-research/tau2-bench/blob/main/src/tau2/data_model/simulation.py), [`scripts/leaderboard/submission.py`](https://github.com/sierra-research/tau2-bench/blob/main/src/tau2/scripts/leaderboard/submission.py).
- **AgentBench** (THUDM) [`src/typings/output.py`, `status.py`](https://github.com/THUDM/AgentBench/tree/main/src/typings) (main, checked 2026-09-18). **GAIA** leaderboard [submission format](https://huggingface.co/spaces/gaia-benchmark/leaderboard).
- OTel crossover: [semantic-conventions-genai #338, "ATIF trajectory format used by Harbor"](https://github.com/open-telemetry/semantic-conventions-genai/issues/338) (2026-06-23).

## Span structure

How each framework structures a run / sample / epoch and its transcript events.

| Framework | Top-level unit | Per-sample unit | Transcript model | Nesting |
|---|---|---|---|---|
| Inspect AI | `EvalLog` (one task x one model; `eval.run_id`, `eval.eval_id`, `eval.task_id`) | `EvalSample` keyed by `(id, epoch)`, plus `uuid` per sample run | `EvalSample.events: list[Event]`, each with `event` discriminator, `uuid`, `span_id`, `timestamp`, `working_start` | Spans via `SpanBeginEvent{id, parent_id, type, name}` / `SpanEndEvent{id}`; `ToolEvent.events` and `SubtaskEvent.events` nest child transcripts; `ToolEvent.agent_span_id` links handoffs |
| Harbor | Job (`JobStats`) | `TrialResult` (one task x one agent, `trial_name`, `id: UUID`); multi-step tasks add `step_results: list[StepResult]` | ATIF `Trajectory.steps: list[Step]`; one step per LLM inference ("one-LLM-per-step convention"), `source` in `system|user|agent` | Subagents via `ObservationResult.subagent_trajectory_ref` -> `Trajectory.subagent_trajectories` (embedded, each with own `trajectory_id`) or external `trajectory_path`; `continued_trajectory_ref` chains segments after compaction |
| SWE-agent | batch dir (`preds.json`, `run_batch_exit_statuses.yaml`) | `<instance_id>.traj` = `AgentRunResult{info, trajectory}` (+ `history`) | `trajectory: list[TrajectoryStep]` (thought/action/observation) and `history: list[HistoryItem]` (LM messages) | Flat; `HistoryItem.agent` names the sub-agent for multi-agent configs |
| mini-SWE-agent | `preds.json` | `.traj.json` `{info, messages, trajectory_format}` | `messages` in OpenAI chat format, each with `extra{actions, cost, timestamp, response, returncode, exit_status, submission}` | Flat |
| SWE-bench harness | `<model>.<run_id>.json` run report | per-instance `logs/run_evaluation/<run_id>/<model>/<instance_id>/{report.json, run_instance.log, test_output.txt, patch.diff, eval.sh}` | None (grader only) | n/a |
| OpenAI Evals | run = one JSONL file: first line `{"spec": RunSpec}`, last line `{"final_report", "run_id"}` | events with `sample_id` | `Event{run_id, event_id, sample_id, type, data, created_by, created_at}`; `type` in `sampling|match|embedding|function_call|cond_logp|pick_option|raw_sample|metrics|error|extra` | Flat |
| Vivaria | Run (`runs_t`) | `AgentBranch` (`agentBranchNumber`, trunk = 0, branches fork from `parentTraceEntryId`) | `TraceEntry{runId, index, agentBranchNumber, calledAt, content: EntryContent, usageTokens, usageActions, usageTotalSeconds, usageCost, modifiedAt}` | `frameStart{name}` / `frameEnd` entries delimit nested frames; branches form a tree |
| tau2-bench | `Results{info, tasks, simulations, simulation_index}` | `SimulationRun` (`id`, `task_id`, `trial`) | `messages: list[Message]` (text) or `ticks` (voice) | Flat |
| AgentBench | per-task `runs.jsonl` | `TaskOutput{index, status, result, history}` | `history: list[ChatHistoryItem]` | Flat |
| GAIA | JSONL submission | `{task_id, model_answer, reasoning_trace}` | free-text `reasoning_trace` | n/a |

## Attributes

Per-step / per-event fields (per-run fields are in "Per-run summary fields"). Names are exact.

### Inspect AI events (`inspect_ai.event`)

| name | type | set on | required | description | source |
|---|---|---|---|---|---|
| `uuid`, `span_id`, `timestamp`, `working_start`, `metadata`, `pending` | str, str, UtcDatetime, float, dict, bool | every `BaseEvent` | `timestamp`, `working_start` | `working_start` = sample working time at which event occurred | `_base.py` |
| `ModelEvent.model`, `role`, `input`, `tools`, `tool_choice`, `config`, `output: ModelOutput`, `retries`, `error`, `traceback`, `cache: "read"|"write"`, `call: ModelCall{request, response, error, time}`, `completed`, `working_time` | | `event="model"` | `model`, `input`, `output` | `working_time` = "working time for model call that succeeded (i.e. was not retried)"; `call` holds raw provider request/response when `log_model_api` | `_model.py`, `_model_call.py` |
| `ModelOutput.model`, `choices[].stop_reason`, `usage: ModelUsage`, `fallback: ModelFallback{model, fallback_model, count}`, `time`, `error` | | ModelEvent.output | | `StopReason` = `stop|max_tokens|model_length|tool_calls|content_filter|unknown` | `_model_output.py` |
| `ModelUsage.input_tokens`, `output_tokens`, `total_tokens`, `input_tokens_cache_write`, `input_tokens_cache_read`, `reasoning_tokens`, `total_cost` | int, int, int, int?, int?, int?, float? | ModelOutput.usage; `EvalSample.model_usage: dict[str, ModelUsage]` keyed by model | first three | `input_tokens` "charged at full rate (excludes cached tokens)" | `_model_output.py` |
| `ToolEvent.id`, `function`, `arguments`, `result`, `truncated: (from,to)`, `error: ToolCallError`, `events`, `completed`, `working_time`, `agent`, `agent_span_id`, `failed`, `message_id` | | `event="tool"` | `id`, `function`, `arguments` | `working_time` = "time not spent waiting on semaphores" | `_tool.py` |
| `SandboxEvent.action: "exec"|"read_file"|"write_file"`, `cmd`, `options`, `file`, `input`, `result: int`, `output`, `completed` | | `event="sandbox"` | `action` | `input`/`output` "Truncated to 100 lines"; `result` is exit code | `_sandbox.py` |
| `SampleLimitEvent.type`, `message`, `limit` | Literal, str, float? | `event="sample_limit"` | `type`, `message` | | `_sample_limit.py` |
| `InterruptEvent.source: "user_cancel"|"limit"|"system"`, `interrupted: "generate"|"tool_call"|"between_turns"`, `interrupted_tool_call_id`, `interrupted_model_event_id` | | `event="interrupt"` | | records what was in flight when the sample was cut short | `_interrupt.py` |
| `ErrorEvent.error: EvalError{message, traceback, traceback_ansi}` | | `event="error"` | | | `_error.py` |
| `ScoreEvent.score: Score{value, answer, explanation, reason, metadata, history}`, `target`, `intermediate`, `scorer`, `scorer_args`, `model_usage`, `role_usage` | | `event="score"` | `score` | cumulative usage snapshot at score time | `_score.py` |
| `StateEvent.changes`, `StoreEvent.changes` | list[JsonChange] (JSON Patch) | `event="state"|"store"` | | diffs, not snapshots | `_state.py`, `_store.py` |
| `CompactionEvent.type: "summary"|"edit"|"trim"`, `role`, `tokens_before`, `tokens_after`, `source` | | `event="compaction"` | | | `_compaction.py` |
| `ApprovalEvent.approver`, `decision: approve|modify|reject|escalate|terminate`, `call`, `modified`, `explanation`; `ReviewEvent.reviewer`, `decision: continue|terminate|escalate` | | | | human/auto approval of tool calls | `_approval.py`, `_review.py` |
| Others: `sample_init{sample, state}`, `input`, `logger{message: LoggingMessage}`, `info{source, data}`, `step`, `subtask{name, input, result, events, working_time}`, `span_begin`, `span_end`, `checkpoint`, `anchor`, `branch`, `score_edit` | | | | | `event/` |

### Harbor ATIF step (`Step`, RFC 0001 v1.8)

| name | type | required | description |
|---|---|---|---|
| `step_id` | int >= 1 | yes | ordinal |
| `timestamp` | ISO 8601 str | no | |
| `source` | `"system"|"user"|"agent"` | yes | |
| `model_name`, `reasoning_effort` | str; str or float | no | override root `agent.model_name` |
| `message` | str or `list[ContentPart{type: text|image|audio, text, source}]` | yes | may be empty string |
| `reasoning_content` | str | no | |
| `tool_calls` | `list[ToolCall{tool_call_id, function_name, arguments, extra}]` | no | |
| `observation` | `Observation{results: list[ObservationResult{source_call_id, content, subagent_trajectory_ref, extra}]}` | no | `source_call_id` correlates to `tool_call_id` |
| `metrics` | `Metrics{prompt_tokens, completion_tokens, cached_tokens, cost_usd, prompt_token_ids, completion_token_ids, logprobs, extra}` | no | `cached_tokens` is a subset of `prompt_tokens`; provider extras (e.g. `reasoning_tokens`, `cache_creation_input_tokens`) go in `extra` |
| `llm_call_count` | int >= 0 | no | 0 = deterministic dispatch, >1 = aggregated metrics |
| `is_copied_context` | bool | no | steps copied across a compaction boundary; SFT consumers MUST filter |

### SWE-agent step (`TrajectoryStep` / `StepOutput`)

| name | type | description |
|---|---|---|
| `action`, `observation`, `response`, `thought` | str | parsed LM output and env result |
| `state` | dict[str,str] | env state after step, e.g. `open_file`, `working_dir` |
| `execution_time` | float | seconds for the action |
| `query` | list[dict] | exact LM input at this step (replaced `messages` in 1.1.0) |
| `extra_info` | dict | |
| `StepOutput` only: `done`, `exit_status`, `submission`, `tool_calls`, `tool_call_ids`, `thinking_blocks` | | |

### Vivaria trace entry content (`EntryContent`, discriminated on `type`)

| `type` | fields |
|---|---|
| `generation` | `agentRequest: GenerationRequest{settings, messages|prompt, functions}`, `finalResult: MiddlemanResult`, `requestEditLog[]`, passthrough variants |
| `action` / `observation` | `action: record`, `observation: record` (agent-defined) |
| `log` | `content: any[]`, `attributes{style, title}` |
| `error` | `from: ErrorSource = agent|server|task|serverOrTask|user|usageLimits`, `detail`, `trace`, `extra` |
| `submission` | `value: str` |
| `intermediateScore` | `score: number|NaN|Infinity|-Infinity|null`, `message`, `details` |
| `burnTokens` | `finalResult{n_prompt_tokens_spent, n_completion_tokens_spent, n_serial_action_tokens_spent}` |
| `frameStart{name}`, `frameEnd`, `agentState`, `input`, `rating`, `settingChange`, `safetyPolicy` | |

Every `TraceEntry` also carries cumulative `usageTokens`, `usageActions`, `usageTotalSeconds`, `usageCost` at the time of the entry.

### OpenAI Evals event data

| `type` | `data` keys |
|---|---|
| `sampling` | `prompt`, `sampled`, plus `**extra` (the OpenAI completion fn passes `usage=result.raw_data.usage`) |
| `match` | `correct: bool`, `expected`, `picked`, `**extra` |
| `function_call` | `name`, `arguments`, `return_value` |
| `error` | `type` (exception class name), `message` |
| `metrics` | arbitrary kwargs |

## Metrics

Metrics and scores each framework computes.

| name | instrument | unit | dimensions | description | source |
|---|---|---|---|---|---|
| `EvalResults.scores[].metrics{name, value, params}` | aggregate | scorer-defined (accuracy, mean, stderr, ...) | scorer, reducer | per `EvalScore{name, scorer, reducer, scored_samples, unscored_samples}`; `headline: HeadlineMetric` | Inspect |
| `EvalResults.total_samples`, `completed_samples`, `logged_samples`, `early_stopping` | counters | samples | | `total_samples` = dataset x epochs | Inspect |
| `EvalStats.model_usage`, `role_usage: dict[str, ModelUsage]` | sum | tokens, USD | model name / role | eval-level totals | Inspect |
| `reductions[].samples[]` (`EvalSampleReductions{scorer, reducer}`) | reduce over epochs | | | multi-epoch reduction (mean, max, ...) | Inspect |
| `JobStats.n_completed_trials`, `n_errored_trials`, `n_running_trials`, `n_pending_trials`, `n_cancelled_trials`, `n_retries`, `n_input_tokens`, `n_cache_tokens`, `n_output_tokens`, `cost_usd` | counters | trials, tokens, USD | | | Harbor `models/job/result.py` |
| `AgentDatasetStats.n_trials`, `n_errors`, `pass_at_k: dict[int,float]`, `reward_stats`, `exception_stats`, `metrics` | | | agent x model x dataset (`agent__model__dataset` key) | `exception_stats: dict[exception_type, list[trial_name]]` | Harbor |
| `total_instances`, `submitted_instances`, `completed_instances`, `resolved_instances`, `unresolved_instances`, `infra_failure_instances`, `ambiguous_failure_instances`, `empty_patch_instances`, `error_instances` + matching `*_ids` lists, `schema_version` | counters | instances | | run report | SWE-bench `reporting.py` |
| `GlobalStats.total_cost`; `run_batch_exit_statuses.yaml` / `instances_by_exit_status` | sum, histogram | USD, count | exit_status | | SWE-agent |
| `final_report` (e.g. `accuracy`), `metrics` events | | eval-defined | | last JSONL line | OpenAI Evals |
| `DomainResults.pass_1..pass_4`, `cost`; `InteractionMetrics` (voice) | | pass^k, USD | domain (`retail`, `airline`, `telecom`, `banking_knowledge`) | leaderboard submission | tau2-bench |
| `AgentBranch.score`; `RunUsage{tokens, actions, total_seconds, cost}` vs `usageLimits` | gauge | | branch | | Vivaria |
| `model_answer` exact-match score | | | | computed server-side | GAIA |

## Events and log records

Content-capture and log-ish records.

| name | carried on | fields | when emitted / opt-in | source |
|---|---|---|---|---|
| `ModelEvent.call` (raw API request/response) | transcript event | `ModelCall{request, response, error, time, call_key}` | only when `EvalConfig.log_model_api` is set | Inspect |
| `EvalConfig.log_images`, `log_samples`, `log_realtime`, `log_buffer`, `log_shared` | config | bool/int | image base64 capture and event streaming are opt-in | Inspect |
| `LoggerEvent.message` | transcript event | `LoggingMessage{name, level, message, created, filename, module, lineno}` | Python log records above threshold are folded into the sample transcript | Inspect |
| `EvalSample.attachments` | sample | `dict[str,str]` | large content (images) is de-duplicated by hash and referenced from messages/events; `events_data` pools message content | Inspect |
| `SandboxEvent` | transcript event | see above | every `sandbox().exec/read_file/write_file`; truncated to 100 lines | Inspect |
| `trajectory.json` (ATIF) | file in `<trial>/agent/` | `Trajectory` | written by integrated agents; `AgentConfig.include_logs`/`exclude_logs` glob-filter what agent logs are downloaded | Harbor |
| `verifier/test-stdout.txt`, `test-stderr.txt`, `reward.txt` or `reward.json` | files | reward is `float` (text) or `dict[str, float|int]` (json), non-finite rejected | after verifier runs | Harbor `verifier.py` |
| `artifacts/manifest.json` | file | `ArtifactConfig` collected paths | env teardown | Harbor |
| `<instance_id>.{debug,info,warning}.log`, `config.yaml` | files per instance | | always | SWE-agent |
| `run_instance.log`, `test_output.txt` (with `>>>>> Start Test Output`, `>>>>> Tests Timed Out`, `>>>>> Patch Apply Failed`, `>>>>> Reset Failed`, `>>>>> Tests Errored` markers) | files | | grader | SWE-bench |
| `Event` JSONL line | log record | `run_id, event_id, sample_id, type, data, created_by, created_at`; `hidden_data_fields` redacts keys on write | flushed every `MIN_FLUSH_EVENTS` / `MIN_FLUSH_SECONDS` | OpenAI Evals |
| `TraceEntry` row | DB row | see above | pyhooks `log`, `action`, `observation`, `generate`, `submit`, `log_error` calls | Vivaria |

## Per-run summary fields

What each source records once per sample / trial / run.

### Inspect AI `EvalSample`

| field | type | notes |
|---|---|---|
| `id`, `epoch`, `uuid` | int|str, int, str | `uuid` is globally unique per sample run |
| `input`, `choices`, `target`, `sandbox: SandboxEnvironmentSpec`, `files`, `setup`, `metadata` | | dataset side |
| `messages`, `output: ModelOutput`, `store`, `events`, `timelines` | | final conversation and transcript |
| `scores: dict[str, Score]` | `Score{value, answer, explanation, reason, metadata, history}` | keyed by scorer name; `history` records `ScoreEdit`s |
| `model_usage`, `role_usage: dict[str, ModelUsage]`, `model_fallbacks` | | per-model token totals |
| `started_at`, `completed_at`, `total_time`, `working_time` | UtcDatetimeStr, float s | `working_time` = clock time minus failed/retried generations and waits on shared resources |
| `turn_count`, `message_count` (summary only) | int | turn = one top-level `generate()` after retries/fallbacks |
| `limit: EvalSampleLimit{type, limit, reason}` | `type` in `context|time|working|message|token|turn|cost|operator|custom` | the limit that halted the sample |
| `token_limit`, `token_limit_type`, `token_limit_usage`, `message_limit`, `time_limit` | | ceilings and metered usage copied onto the sample |
| `error: EvalError{message, traceback, traceback_ansi}` | | error that halted the sample |
| `error_retries: list[EvalRetryError{message, traceback, traceback_ansi, events}]` | | each retried attempt keeps its own events |
| `invalidation: ProvenanceData` | | post-hoc invalidation |

`EvalLog.status` is `started|success|cancelled|error`; `EvalLog.error` holds the eval-level `EvalError`. Config knobs: `fail_on_error: bool|float`, `continue_on_fail`, `retry_on_error: int`, `score_on_error`. Limits do not error: "samples that encounter limits are still scored, albeit nearly always as incorrect".

### Harbor `TrialResult`

| field | type | notes |
|---|---|---|
| `id: UUID`, `task_name`, `trial_name`, `trial_uri`, `task_id: LocalTaskId|GitTaskId|PackageTaskId`, `source`, `task_checksum` | | `task_checksum` pins task content |
| `config: TrialConfig` | | full agent/env/verifier config, `env` secrets templatized on serialization |
| `agent_info: AgentInfo{name, version, model_info: ModelInfo{name, provider}}` | | |
| `agent_result: AgentContext{n_input_tokens, n_cache_tokens, n_output_tokens, cost_usd, model_usage: dict[str, ModelUsage], rollout_details, metadata}` | | `n_input_tokens` "including cache"; `model_usage` "backfilled from the ATIF trajectory when the agent does not populate it" |
| `verifier_result: VerifierResult{rewards: dict[str, float|int]}` | | `reward.txt` -> `{"reward": v}` |
| `verifier_environment_mode` | `shared|separate` | |
| `exception_info: ExceptionInfo{exception_type, exception_message, exception_traceback, occurred_at}` | | timeouts are `AgentTimeoutError`, `AgentSetupTimeoutError`, `VerifierTimeoutError`, `EnvironmentStartTimeoutError` |
| `started_at`, `finished_at`, `environment_setup`, `agent_setup`, `agent_execution`, `verifier: TimingInfo{started_at, finished_at}` | | phase timings |
| `step_results: list[StepResult{step_name, agent_result, verifier_result, exception_info, agent_execution, verifier}]` | | multi-step tasks |

ATIF root: `schema_version` ("ATIF-v1.8"), `session_id` (run-scoped), `trajectory_id` (document-scoped), `agent{name, version, model_name, tool_definitions, extra}`, `steps`, `final_metrics{total_prompt_tokens, total_completion_tokens, total_cached_tokens, total_cost_usd, total_steps, extra}`, `notes`, `continued_trajectory_ref`, `subagent_trajectories`, `extra`.

### SWE-agent `AgentInfo` (`.traj` `info` block)

| field | type | notes |
|---|---|---|
| `exit_status` | str | `"submitted"`, `"exit_command"`, `"early_exit"`, exception-derived values (`ContextWindowExceededError`, `InstanceCostLimitExceededError`, `InstanceCallLimitExceededError`, `TotalCostLimitExceededError`, `ContentPolicyViolationError`, `FormatError`, ...) |
| `submission` | str|None | final patch; `preds.json` maps to `{instance_id, model_name_or_path, model_patch}` |
| `model_stats` | `InstanceStats{instance_cost, tokens_sent, tokens_received, api_calls}` | cumulative across all API calls for the instance |
| `review`, `summarizer`, `edited_files30/50/70` | | optional |
| `swe_agent_version`, `swe_agent_hash`, `swe_rex_version`, `swe_rex_hash` | str | tool version + git hash |

mini-SWE-agent: `info{model_stats{instance_cost, api_calls}, config{agent, agent_type, model, model_type, environment, environment_type}, mini_version, exit_status, submission}`, `trajectory_format: "mini-swe-agent-1.1"`; `exit_status` in `Submitted|LimitsExceeded|TimeExceeded|RepeatedFormatError|<ExceptionName>`.

### SWE-bench per-instance `report.json`

`{instance_id: {patch_is_None, patch_exists, patch_successfully_applied, resolved, infra_failure, infra_failure_reason, tests_status: {FAIL_TO_PASS: {success[], failure[]}, PASS_TO_PASS: {...}, FAIL_TO_FAIL, PASS_TO_FAIL}}}`. Resolution is `RESOLVED_FULL|RESOLVED_PARTIAL|RESOLVED_NO`; `resolved` is true only for FULL. Timeout is a harness flag (`--timeout`), appended as "Timeout error: N seconds exceeded" in `test_output.txt`.

### OpenAI Evals

`RunSpec{completion_fns, eval_name, base_eval, split, run_config, created_by, run_id, created_at}` then per-sample events, then `{"final_report": {...}, "run_id"}`. No per-sample summary object; consumers join events by `sample_id`.

### Vivaria `AgentBranch`

`runId`, `agentBranchNumber`, `parentAgentBranchNumber`, `parentTraceEntryId`, `submission`, `score`, `fatalError: ErrorEC`, `usageLimits: RunUsage{tokens, actions, total_seconds, cost}`, `checkpoint: UsageCheckpoint`, `scoreCommandResult`, `agentCommandResult: ExecResult{stdout, stderr, exitStatus}`, `agentPid`, `agentStartingState`, `agentSettings`, `createdAt`, `startedAt`, `completedAt`, `isRunning` ("true iff submission or fatalError are set" -- sic), `isInteractive`, `isInvalid`. `RunStatus` in `concurrency-limited|error|killed|manual-scoring|paused|queued|running|setting-up|submitted|usage-limits`. Branch usage limits exclude ancestor usage.

### tau2-bench `SimulationRun`

`id`, `task_id`, `trial`, `seed`, `timestamp`, `start_time`, `end_time`, `duration`, `termination_reason` (`user_stop|agent_stop|max_steps|timeout|too_many_errors|agent_error|user_error|infrastructure_error|context_window_exceeded|unexpected_error`), `agent_cost`, `user_cost`, `agent_usage: SessionUsage{records: list[UsageRecord], cost, cost_breakdown, pricing_version}`, `reward_info: RewardInfo{reward, db_check, env_assertions, action_checks, nl_assertions, communicate_checks, reward_basis, reward_breakdown, info}`, `messages`, `review`, `hallucination_retries_used`, `provider_session_id`. `Results.info: Info{git_commit, num_trials, max_steps, max_errors, user_info, agent_info{implementation, llm, llm_args}, environment_info, seed}`.

### AgentBench / GAIA

AgentBench `TaskOutput{index, status: SampleStatus, result, history}` with `SampleStatus` in `running|completed|agent context limit|agent validation failed|agent invalid action|task limit reached|unknown|task error`; `AgentOutputStatus` in `normal|cancelled|agent context limit`. GAIA requires only `task_id`, `model_answer`, optional `reasoning_trace`; run metadata (model name, family, org) is entered in the submission form, not the record.

## Execution environment

| Framework | Git / source | Sandbox / container | Resources | Wall time | Versions / seeds |
|---|---|---|---|---|---|
| Inspect AI | `EvalSpec.revision: EvalRevision{type: "git", origin, commit, dirty}`, `task_file` | `EvalSpec.sandbox` and `EvalSample.sandbox: SandboxEnvironmentSpec` (type + config, e.g. compose file); `sandbox_cleanup`, `sandbox_prebuilt`, `max_sandboxes` | not recorded per sample (only concurrency caps `max_samples`, `max_subprocesses`) | `EvalStats.started_at/completed_at`; per sample `total_time`, `working_time` | `EvalSpec.packages: dict[str,str]`, `task_version`, `model_generate_config` (incl. seed) |
| Harbor | `task_id: GitTaskId` (repo + ref), `task_checksum` | `EnvironmentConfig{type, cpus, memory_mb, storage_mb, gpus, tpu, mounts, extra_docker_compose, cpu/memory_enforcement_policy}`; per-phase network allowlists | declared, not measured | `TimingInfo` per phase | `agent_info.version`, `config` dump |
| SWE-agent | `swe_agent_hash`, `swe_rex_hash` | `config.yaml` (deployment: docker image etc.) | not measured | `execution_time` per step only | `swe_agent_version`, `swe_rex_version` |
| SWE-bench | instance fields (`repo`, `base_commit`, `environment_setup_commit`) in dataset, not report | docker image per instance | none | `total_runtime` from `exec_run_with_timeout` (logged, not in report) | harness version implicit |
| Vivaria | `RunTableRow.agentRepoName`, `agentBranch`, `agentCommitId`, `taskCommitId`, `taskBranch`, `serverCommitId` | `taskBuildCommandResult`, `containerCreationCommandResult`, `agentBuildCommandResult`, `taskStartCommandResult`, `auxVmBuildCommandResult: ExecResult`; Task Standard `VMSpec{cpu_count_range, ram_gib_range, gpu_spec}` for aux VMs | usage measured as `tokens`, `actions`, `total_seconds`, `cost` (limits + checkpoints) | `startedAt`, `completedAt` | `agentSettingsPack`, `standard_version` |
| tau2-bench | `Info.git_commit` | none | none | `duration` | `seed`, `pricing_version` |
| OpenAI Evals | none | none | none | `created_at` per event | `RunSpec.run_config` |

No framework records host CPU/memory consumption per sample; Vivaria is the only one with a first-class "actions" counter and a cost/seconds ledger on every trace entry.

## Notable design choices

- **Inspect keeps two clocks.** `total_time` vs `working_time` (excluding rate-limit retries and sandbox waits) and a separate `working_limit`; every event carries `working_start`. Copy this: wall time alone penalizes the harness, not the agent.
- **Inspect splits "limit" from "error".** A limit is a normal early exit (`EvalSample.limit`, still scored); an error is `EvalSample.error` plus `error_retries[]` holding the events of each failed attempt. `InterruptEvent` records what was in flight (`generate|tool_call|between_turns`).
- **Inspect token limits are expression-typed** (`token_limit_type` = `"output"` or a formula over `input`/`output`), and `turn_count` is distinct from `message_count`.
- **Harbor separates trial outcome from trajectory.** `result.json` carries tokens/cost/rewards/exception; `trajectory.json` (ATIF) carries content. `AgentContext.model_usage` is backfilled from ATIF when the agent does not report it. Phase timings (`environment_setup`, `agent_setup`, `agent_execution`, `verifier`) are separate `TimingInfo`s.
- **ATIF cache accounting:** `cached_tokens` is a subset of `prompt_tokens` (Harbor `n_input_tokens` "including cache"). Inspect's `input_tokens` **excludes** cached tokens. Any mapping must state which convention it uses.
- **ATIF has no per-step timing beyond `timestamp`, no exit code on observations, no error field**: errors surface as observation content or `extra`. OTel issue #338 (2026-06) lists cost attributes, cache/reasoning token breakdown, and causal span links for parallel tools as P1 gaps in the GenAI conventions before ATIF <-> OTel can be lossless.
- **Rewards are dicts, not scalars**, in Harbor (`rewards: dict[str, float|int]`), tau2 (`reward` + `reward_breakdown`), Inspect (`scores: dict[scorer, Score]`). SWE-bench's `resolved` bool is derived from a graded `tests_status`.
- **Stop reasons are strings from a closed set** everywhere except Inspect (typed `EvalSampleLimit.type` + `StopReason`). SWE-agent leaks Python exception class names into `exit_status`; tau2 and AgentBench enumerate them.
- **Cost accounting across retries**: Inspect counts a retried generation's tokens in `model_usage` but excludes its time from `working_time`; `ModelEvent.retries` is per call. SWE-agent's `InstanceStats` is a running sum of every API call (`api_calls`) including failures that returned usage. Vivaria charges every `generation` entry to the branch ledger. None separates "billed on failed attempts" as its own field.
- **Redaction**: OpenAI Evals `hidden_data_fields`; Harbor `templatize_sensitive_env` on config serialization; Inspect `log_model_api` and `log_images` off by default, sandbox I/O truncated to 100 lines.
- **Correlation ids**: Inspect `eval_set_id > eval_id / run_id > task_id > sample uuid > event uuid / span_id`; ATIF `session_id` (run) vs `trajectory_id` (document); Vivaria `runId + agentBranchNumber + index`.

## Recommendation for lablet (opinion)

1. **Outcome fields present in most frameworks -- adopt**: `run_id` (uuid), `task_id`, `started_at`, `completed_at`, `total_time_s`, `working_time_s`, `turn_count`, `message_count`, `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `reasoning_tokens`, `cost_usd`, `stop_reason` (closed enum), `error {type, message, traceback}`, `submission`, `score(s)`. These map 1:1 onto Inspect `EvalSample`, Harbor `TrialResult`/`AgentContext`, SWE-agent `info`, tau2 `SimulationRun`.
2. **Split `stop_reason` into limit vs error like Inspect -- adopt**: `limit {type: context|time|working|message|token|turn|cost|operator, limit, usage}` distinct from `error`. Every framework surveyed has both concepts but only Inspect models them cleanly; make the enum a superset of Inspect's and tau2's.
3. **Two clocks -- adopt**: emit `total_time` and `working_time` (exclude provider retries and sandbox waits) and stamp `working_start` on transcript events; otherwise lablet cannot feed Inspect's `working_limit` semantics.
4. **State the cache convention explicitly -- adapt**: carry both `input_tokens` (uncached, Inspect) and `prompt_tokens` (incl. cache, ATIF/Harbor) or one plus `cache_read_tokens` with a documented formula. Ambiguity here is the single most likely composition bug.
5. **Retry accounting -- adopt with an addition**: cumulative usage includes failed attempts (all frameworks), plus a per-call `retries` count (Inspect `ModelEvent.retries`) and a lablet-only `billed_failed_attempt_tokens` so evaluators can subtract if desired.
6. **Transcript should round-trip to ATIF v1.8 -- adopt**: one lablet turn = one ATIF `Step` (`source: agent`, `tool_calls[]`, `observation.results[]` with `source_call_id`, `metrics{prompt_tokens, completion_tokens, cached_tokens, cost_usd}`, `llm_call_count: 1`). Put exit codes, durations, and truncation flags in `ObservationResult.extra` and `ToolCall.extra` since ATIF has no slot for them. This gives Harbor/Terminal-Bench and Opik import for free.
7. **Also map to Inspect events -- adapt**: lablet's turn/model/tool/sandbox events correspond to `ModelEvent`, `ToolEvent`, `SandboxEvent{action, cmd, result, output}`, `SampleLimitEvent`, `ErrorEvent`; keep `SandboxEvent`-style `exec` records (cmd, exit code, truncated output) as first-class since ATIF lacks them.
8. **Scores as a named map -- adopt**: `scores: {name: {value, explanation, metadata}}` (Inspect `Score`, Harbor `rewards`, tau2 `reward_breakdown`), not a single float; lablet is not the grader, so leave this nullable and let the composer fill it.
9. **Environment provenance -- adopt Inspect + Harbor union**: `revision {origin, commit, dirty}`, `sandbox {type, image, config_hash}`, declared `cpus/memory_mb`, `packages/versions` (lablet version + model id + provider). Skip measured host CPU/RSS: no surveyed framework records it and it is not comparable across hosts.
10. **Do not adopt**: OpenAI Evals' untyped `type/data` event bag (archived, no agent semantics); SWE-agent's exception-class-name `exit_status`; GAIA's free-text `reasoning_trace` as a transcript format. Skip per-token ids/logprobs (ATIF `prompt_token_ids`, `logprobs`) unless an RL consumer appears.
