# lablet

**L**ightweight **A**gent **B**enchmarking **L**oop for **E**valuation & **T**esting.

A lablet is one program that runs one instrumented agent loop: a model, a set of tools, a prompt, and stop conditions in; an outcome and OpenTelemetry traces out. It is designed to be composed into larger evaluation and testing frameworks, for example to optimise an MCP server for token usage and tool calls.

| Area | Contents |
| --- | --- |
| [product/](product/) | What we are building and why: brief, spec, build plan, decisions |
| [contributing/](contributing/) | How to work in this repository |
| [lablet/](lablet/) | The Rust workspace and user-facing documentation |
