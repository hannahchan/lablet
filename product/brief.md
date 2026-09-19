# Lablet

**L**ightweight **A**gent **B**enchmarking **L**oop for **E**valuation & **T**esting.

## The problem

Optimising the things around an agent (an MCP server's tool surface, a skill, a system prompt, a model choice) needs a fast, repeatable way to answer "how many tokens and tool calls did it take to finish this task, and where did they go?" Full agent products bundle the loop with a harness, opinions, and telemetry you can't shape. Eval frameworks are heavy and own the whole run.

## The idea

A lablet is one program that runs one agent loop. You give it:

- a model (Anthropic, or anything speaking the OpenAI chat completions API, such as Ollama),
- a set of tools (MCP servers plus optional built-ins, each individually switchable),
- a system prompt and optional inlined skills,
- a task prompt,
- stop conditions (completion, turns, timeout, retries).

It runs the loop to completion inside whatever execution environment it was started in (a container, a sandbox, a laptop) and emits standardised OpenTelemetry traces and logs following the GenAI semantic conventions over OTLP. The final result goes to stdout as JSON.

That's all it does. Containers, task suites, grading, repetition, and analysis belong to the larger framework that composes lablets. A grader is just another lablet with a different config.

## Example use case

Optimise an MCP server: run the same task suite against versions of the server, collect the OTLP traces, and compare tokens, tool calls, and wall time per task. Remove a tool from the config to see what the agent does without it.

## Out of scope

- Orchestrating containers or environments.
- Grading or scoring results.
- Hosting MCP servers.
- Writing Arrow or Parquet directly. The collector side does that.
- Multi-agent coordination inside one process. One lablet, one loop.

## Design commitments

- **Rust**, to sit alongside the Apache Arrow and DataFusion ecosystem the analysis side will use.
- **Explicit architecture**: domain, application, adapters, composition root, with the layer rules enforced by a lint.
- **Own loop, not a wrapped SDK**, so every step is instrumented on our terms.
- **CLI and library** from the same composition root.
- **Raw data, never reports.** One lablet's telemetry matters because it joins with ten thousand others. Every record is shaped for aggregation by the composing system; lablet itself never aggregates, reports, or visualises.
