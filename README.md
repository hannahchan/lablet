# lablet

**L**ightweight **A**gent **B**enchmarking **L**oop for **E**valuation & **T**esting.

A lablet is one program that runs one instrumented agent loop: a model, a set of tools, a prompt, and stop conditions in; an outcome and OpenTelemetry traces out. It's designed to be composed into larger evaluation and testing frameworks, for example to optimise an MCP server for token usage and tool calls.

| Area | Contents |
| --- | --- |
| [product/](product/) | What we're building and why: brief, spec, build plan, decisions |
| [contributing/](contributing/) | How to work in this repository |
| [lablet/](lablet/) | The Rust workspace and user-facing documentation |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
