# Parity research

How lablet's loop compares with real agent loops, so that what lablet measures carries over to how agents actually run. Researched 2026-09-24 and 2026-09-25. Start with the matrix. The reference documents hold the detail and the citations.

## Why lablet has references

A loop's choices change what it measures. One study ran the same tasks with the same models through three harnesses, and found about a 40-fold difference in tokens per solved task between two of them ([arXiv 2607.22585](https://arxiv.org/abs/2607.22585)). So lablet names the loops it aims to behave like, and takes its defaults from them. It doesn't aim to be identical to any of them. Wherever lablet differs, the difference is recorded.

## References

| Role                             | Loop                                          | Pinned at                                | Why                                                                                                                                                                                                                                                                                                          |
| -------------------------------- | --------------------------------------------- | ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Primary, and the default profile | Claude Code, run through the Claude Agent SDK | Claude Code 2.1.281, released 2026-09-23 | It's the most used coding agent at work, at about 39% of professional developers in JetBrains' 2026 survey, and it had the most identified users on the one MCP server with public client data. It can be run with a custom system prompt and only the tools under test, and its raw requests can be logged. |
| Secondary, for OpenAI models     | Codex CLI                                     | rust-v0.156.1, released 2026-09-23       | It's OpenAI's own loop and it's open source. That makes it the best evidence for how OpenAI models are driven through the Responses API. Parity with it is partial.                                                                                                                                          |

Six more loops are evidence where the references are silent or split: mini-swe-agent, OpenCode, GitHub Copilot CLI, Goose, OpenHands and Gemini CLI. A screen of thirteen others is summarised in [field.md](field.md).

mini-swe-agent was chosen as a minimal baseline profile on 2026-09-25 and made evidence on 2026-09-28. It can only offer a bash tool, its way of completing a run is its own, and anyone can run it directly.

Each reference is compared in a stated configuration, not as it ships. That configuration uses a custom system prompt and offers only the tools under test. It also turns off what lablet deliberately doesn't do: sub-agents, hooks, permission prompts and interactive features. Each reference document gives its configuration.

## What lablet copies, and what it doesn't

Lablet's users compare variants: two tool surfaces, two skills, two prompts, two models. What has to carry over to real use is which variant wins, and by roughly how much. That needs lablet to match the behaviours that bear on the thing being varied, and no others. Every behaviour lablet copies also has to be checked again whenever a reference changes, and the primary ships almost daily.

Two rules set the line (`decisions.md`, 2026-09-28).

1. **What lablet builds.** A behaviour of a reference is built only when all four hold:
   - it changes what the model is sent or how a run proceeds
   - it bears on something a user varies
   - it's a pattern, found in the primary and at least one other reference, or in most of the loops surveyed
   - it can be specified from public evidence and checked against a recorded request
2. **A profile sets values.** A profile configures behaviours lablet has for other reasons. It's never the reason to add one.

A question about one harness is answered by running that harness:

| The question                                          | The tool                                     |
| ----------------------------------------------------- | -------------------------------------------- |
| Which variant is better, and where did the tokens go? | Lablet                                       |
| Same loop, different model?                           | Lablet                                       |
| What does switching one behaviour on or off do?       | Lablet                                       |
| What will a given harness do with my server or skill? | That harness, in its reference configuration |
| How does a feature only one harness has behave?       | That harness                                 |

Known differences, outside the line and not built:

- permissions, hooks, sub-agents, memory and project files
- context a harness injects: the date, the working directory, git status, reminders
- prompts, tools and limits tuned for each model
- Codex's code mode, where the model calls tools by writing JavaScript
- switching models after a refusal
- moving a timed-out command to the background
- a tool list that changes during a run
- keys in a tool's `_meta` that belong to one vendor

Tool search is inside the line and not built yet. The three most used harnesses hide MCP tool schemas until the model searches for them, which decides whether a tool is found at all. Until lablet has it, it's the largest known difference.

## Profiles and settings

A profile is a named set of setting values that brings lablet close to a reference, and the default profile follows the primary reference. A user can override any single value. A setting exists only where the references behave differently on something inside the line, or where an experiment needs that factor isolated.

The profile doesn't change when the model changes. A model comparison changes only the model. To run each model the way its vendor's harness would, a user chooses that profile explicitly. Most multi-provider harnesses tune their loop for each model, and lablet deliberately doesn't.

## How parity is checked

1. **Documented behaviour.** Each matrix row records what each reference does, with its source.
2. **Request shape.** Captured requests from a reference are compared with what lablet sends for the same conversation. Claude Code is captured with its raw request logging rather than through a proxy, because pointing it at a non-Anthropic base URL changes some of its defaults.
3. **Outcomes.** The same model, prompt, tools and tasks are run through a reference and through lablet. Their distributions of turns, tokens, tool calls and success are compared against the reference's own run-to-run spread.
4. **Transfer.** The same comparison of two variants is run in lablet and in a reference. Where the direction and rough size of the effect agree, no more fidelity is added for that kind of question. Where they disagree, the behaviour responsible is found and put to the first rule.

The matrix is a list of suspects for the fourth check, not a list of work. Rows that only a capture can settle are marked in it.

A capture stays out of the repository. It can hold text that isn't lablet's to publish, such as the wording of a reference's own tools, and paths from the machine it was made on. What a capture settles is written into the matrix and the reference's document, and a test that holds lablet to it is built from lablet's own content.

## Documents

| Document                                     | Covers                                                                                                                          |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| [matrix.md](matrix.md)                       | Every behaviour that affects a measurement: what each reference does, lablet at the survey, and what was decided or is proposed |
| [claude-code.md](claude-code.md)             | The primary reference in detail, with its reference configuration                                                               |
| [codex.md](codex.md)                         | The secondary reference in detail, with its reference configuration                                                             |
| [mini-swe-agent.md](mini-swe-agent.md)       | Evidence: the bash-only harness the SWE-bench leaderboard runs                                                                  |
| [field.md](field.md)                         | The five other evidence loops and the screen of thirteen others                                                                 |
| [shell-environment.md](shell-environment.md) | What the eight loops give the commands and stdio servers they start, and what lablet decided for its own                        |

Research agents compiled the reference documents from the sources they cite. A second pass checked every matrix cell against those documents, and checked the claims the proposals depend on against the primary sources. Each reference document ends with the corrections that pass found. Other claims in the reference documents weren't checked again. Everything in the matrix except its last column is inventory. The last column is opinion.
