# CLAUDE.md

Lablet: a lightweight, instrumented agent loop in Rust, built with explicit architecture.

- What and why: [product/](product/) — read `brief.md`, then `spec.md`, then `build-plan.md`.
- How to work here: [contributing/README.md](contributing/README.md) — layer rules, code conventions, gates.
- The code: [lablet/](lablet/) — Cargo workspace; run plain cargo commands from there, `cargo xtask` from the root.

## Delivery

- Work on a branch. When a logical piece is complete and `cargo xtask pre-push` passes, fast-forward `main` and push. No pull requests for now. After pushing, check the Actions run; a red `main` is fixed before anything else lands.
- One build-plan phase per explicit human go-ahead. Go as far as possible within the phase, stop where a human is needed (API keys, manual sign-off items), and stop at the end of the phase for review.
- Before stopping at a phase end, run a multi-agent code review over the phase's diff, fix what survives verification, then write the phase report: what landed with commit ids, scenarios green, gate results, architectural decisions made, spec clarifications, human sign-off items, open risks, one demo command.
- Architectural decisions may be made without asking. Record each in `product/decisions.md` when it is made and list it in the phase report.
- Commits are signed through 1Password. If signing fails, stop and wait for the human. Never change signing configuration.
- Never handle API keys or other secrets. Prepare the exact command and let the human run it.
- Stay on the task. Do the work or hand it to a subagent.
