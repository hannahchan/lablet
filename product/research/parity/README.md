# Parity research

How lablet's loop compares with real agent loops, so that what lablet measures carries over to how agents actually run. Researched 2026-09-24 and 2026-09-25. Start with the matrix. The reference documents hold the detail and the citations.

## Why lablet has references

A loop's choices change what it measures. One study ran the same tasks with the same models through three harnesses, and found about a 40-fold difference in tokens per solved task between two of them ([arXiv 2607.22585](https://arxiv.org/abs/2607.22585)). So lablet names the loops it aims to behave like, and takes its defaults from them. It doesn't aim to be identical to any of them. Wherever lablet differs, the difference is recorded, and a user can switch the behaviour with a setting.

## References

| Role                             | Loop                                          | Pinned at                                | Why                                                                                                                                                                                                                                                                                                          |
| -------------------------------- | --------------------------------------------- | ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Primary, and the default profile | Claude Code, run through the Claude Agent SDK | Claude Code 2.1.281, released 2026-09-23 | It's the most used coding agent at work, at about 39% of professional developers in JetBrains' 2026 survey, and it had the most identified users on the one MCP server with public client data. It can be run with a custom system prompt and only the tools under test, and its raw requests can be logged. |
| Secondary, for OpenAI models     | Codex CLI                                     | rust-v0.156.1, released 2026-09-23       | It's OpenAI's own loop and it's open source. That makes it the best evidence for how OpenAI models are driven through the Responses API. Parity with it is partial.                                                                                                                                          |
| Minimal baseline                 | mini-swe-agent                                | v2.4.6, released 2026-07-23              | It's the harness the SWE-bench bash-only leaderboard uses to compare models under one fixed prompt. It can only offer a bash tool.                                                                                                                                                                           |

Five more loops are tie-breaking evidence where the references are silent or split: OpenCode, GitHub Copilot CLI, Goose, OpenHands and Gemini CLI. A screen of thirteen others is summarised in [field.md](field.md).

Each reference is compared in a stated configuration, not as it ships. That configuration uses a custom system prompt and offers only the tools under test. It also turns off what lablet deliberately doesn't do: sub-agents, hooks, permission prompts and interactive features. Each reference document gives its configuration.

## Profiles and settings

A profile is a named set of setting values that reproduces a reference, and the default profile follows the primary reference. A user can override any single value. A setting exists only where a reference behaves differently, or where an experiment needs that factor isolated.

The profile doesn't change when the model changes. A model comparison changes only the model. To run each model the way its vendor's harness would, a user chooses that profile explicitly. Most multi-provider harnesses tune their loop for each model, and lablet deliberately doesn't.

## How parity is checked

1. **Documented behaviour.** Each matrix row records what each reference does, with its source.
2. **Request shape.** Captured requests from a reference are compared with what lablet sends for the same conversation. Claude Code is captured with its raw request logging rather than through a proxy, because pointing it at a non-Anthropic base URL changes some of its defaults.
3. **Outcomes.** The same model, prompt, tools and tasks are run through a reference and through lablet. Their distributions of turns, tokens, tool calls and success are compared against the reference's own run-to-run spread.

Rows that only a capture can settle are marked in the matrix.

## Documents

| Document                               | Covers                                                                                                                 |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| [matrix.md](matrix.md)                 | Every behaviour that affects a measurement: what each reference does, lablet today, and a proposed default and setting |
| [claude-code.md](claude-code.md)       | The primary reference in detail, with its reference configuration                                                      |
| [codex.md](codex.md)                   | The secondary reference in detail, with its reference configuration                                                    |
| [mini-swe-agent.md](mini-swe-agent.md) | The minimal baseline in detail                                                                                         |
| [field.md](field.md)                   | The five tie-breaking loops and the screen of thirteen others                                                          |

Research agents compiled the reference documents from the sources they cite. A second pass checked every matrix cell against those documents, and checked the claims the proposals depend on against the primary sources. Each reference document ends with the corrections that pass found. Other claims in the reference documents weren't checked again. Everything in the matrix except its last column is inventory. The last column is opinion.
