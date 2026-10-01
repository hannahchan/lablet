# Research

Background research that informs decisions in the spec. Each folder is one topic; each has its own README.

- [instrumentation/](instrumentation/) — what agent SDKs, observability platforms, eval frameworks, coding agents, and the OpenTelemetry GenAI conventions instrument.
- [weaver/](weaver/) — how to adopt OpenTelemetry Weaver for the telemetry contract: findings, a working spike, the recommended approach, and the fallback.
- [design-review/](design-review/) — the two design reviews of 2026-09-24, kept as delivered: what they found, and the eleven decisions they put to the human.
- [parity/](parity/) — how lablet's loop compares with the agent loops it takes as references: Claude Code, Codex and mini-swe-agent, with a matrix of every behaviour that changes a measurement.
- [secrets/](secrets/) — three deployment stories written to the launch line, against which the secrets design is pressure-tested: what each config and command makes lablet withhold, cut, and leave out of the digest.
