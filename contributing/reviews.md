# Review checklist

What to look for when reviewing a diff here. [README.md](README.md) holds the rules a gate enforces; this page holds the judgements a gate can't make.

Every item comes from a defect that reached `main` in the two domain crates and was caught by a later review. The commit that fixed each one is cited, so the reasoning stays one `git show` away. An item that becomes mechanical moves to `README.md` and becomes a gate.

A reviewer with five findings to spend reads the sections that match the diff. The first is the lens for the rest, not a list of its own.

## Before the review

- Read `product/decisions.md` first. A finding refuted on grounds recorded there costs the same to raise and nothing to settle. In `c75290c`, several of seventeen refuted findings were refuted on entries written in the previous two days.
- Know what the diff claims. The commit message and the spec section it changes are part of the diff, and both are reviewable.

## Structural, not contractual

A rule is held at one of these levels, and the level decides what happens to someone who doesn't know it:

| Level                          | What holds it                                               | From this repository                                                                                                                                                               |
| ------------------------------ | ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Unrepresentable                | The type. The state can't be built.                         | `Message`'s variant is the role, so no message holds a block its role may not send. `UserContent` is text, so a tool block in user input is unrepresentable rather than validated. |
| One door                       | A private field and a constructor that validates.           | `UnknownReason`, whose `From<String>` is the only way in. `Cost`, `Rates`, and the id newtypes, checked on the code path and the serde path alike.                                 |
| Derived                        | The fact is computed where it's needed, never stored twice. | `is_error` reads the call's status. The summary's run totals read the outcome. `Turn::calls(mode)` is the only reading of a response's tool calls.                                 |
| Compile-time, across artefacts | An exhaustive match that fails to build.                    | `spellings.rs`: a new `ToolCallStatus` variant doesn't compile until the registry has the member.                                                                                  |
| Gate                           | `cargo xtask`, in CI.                                       | The layer lint, `weaver check`, the coverage and mutation floors.                                                                                                                  |
| Test                           | One case, one regression.                                   | Most of the suite.                                                                                                                                                                 |
| Convention                     | A sentence in a document.                                   | The port obligation that bounds the per-tool attribute keys, today.                                                                                                                |

**What level is this rule at, and is there a cheaper level up?** The transcript "enforced rules for values by type but left aggregates to convention," pairing a flat message list with a parallel record list by index. Making the pairing a field moved four rules from convention to unrepresentable and deleted the errors that had checked them (`4b6927a`). Deleted error variants are the sign that the move was real.

**Does the change move a level, or only look like it?** Grouping `RunSummary`'s totals into value types reads better and moves nothing: it "doesn't make a swap impossible; only holding the mapping to the generated key list does" (`928dd0b`). Unit newtypes for `Bytes`, `Millis` and `Count` were declined on the same ground, since every attribute is an `int` at the telemetry boundary whatever the domain called it.

**When an invariant moves, does its enforcement level drop?** This is how the per-tool bound was lost. `finish` had gated each entry on `RunSetup::tools`, which bounded the wide event's keys structurally, as a side effect. Reading the call's own status instead was right, "but the bound left with it" and became a sentence about what an adapter must honour (`b970009`). A change can be correct and still cost a level, and nothing fails when it does.

Climbing isn't free, and three refusals are on record. A typestate for the loop protocol was rejected because it "would delete two error variants and cost more than they do." Enforcing that the cache counts are a subset of `input_tokens` was rejected because it would fail a real run over a provider's own arithmetic, and faithful measurement beats internal consistency here. String newtypes for `config_digest` and its neighbours wait for a producer to exist. The cost is paid by everything that touches the type; the benefit is the states that stop existing.

Phase 3 inherits one. The `ToolExecutor` port now states the obligation in its doc, "rather than a rule the model can hold," which is the honest form of the bottom row and not a substitute for leaving it: if resolving a name through the `ToolSet` were the only way to obtain a `ToolSource`, a tool appearing mid-run couldn't produce a `Ran` outcome at all.

## Documentation drift

The most repeated defect in the domain layer's history. Twelve of the fifteen domain commits also changed `product/spec.md`.

1. **When an invariant moves, find every sentence that asserts it.** Type docs, field docs, method docs, spec sections, the decisions log. When the per-tool bound moved from `Run::finish` to the tool executor, the spec drifted in three places, the doc comments in six, and a branch in the summary that had been written when `source()` answered `None` for one reason only (`c75290c`, `f3973d9`). Three of the four were prose; the fourth was behaviour, and no gate saw any of them.
2. **When a type is renamed, grep the old name in prose.** `6ea164d` renamed four types; `c75290c` later found two comments still calling the aggregate root "the tally."
3. **When a signature changes, check the spec's pseudocode.** `a72ab5f` found the loop pseudocode still calling `finish` with five arguments after `rates` made it six.
4. **A doc comment that states a contract is a contract.** `Transcript::record` promised that a refused turn leaves the caller's input alone, and `push` broke it by dropping blank blocks before the refusal (`b970009`). The effect was benign; the sentence was what the loop would be written against.
5. **Re-verify the structural claims a refactor makes.** Prose asserting a structural property drifts as readily as prose asserting an invariant. `d59dfd1` says it dissolves both import cycles in `lablet-model`. It dissolved one, and left `message` and `provider` importing each other through `ProviderKind` and `tool_uses`, until `0ed5744` made the sentence true.
6. **A superseded decision says so where it sits.** The log is read to refuse findings already decided, so an entry that reads as current when it isn't misleads in the one direction that matters. Four entries were reversed by a later one with nothing at the old entry to say so, until `de8432b` marked each in place and said which part still holds.
7. **Ask of any claim: is this still true, or was it true when written?**

## One fact, one place

8. **Can these two fields disagree?** "The model called a tool the run doesn't have" was three facts that could: a missing source, a status, and a name that `finish` looked up a second time in the run's tool list.
9. **Is a fact looked up twice by different routes?** If so, name the authoritative one and say what happens when they differ.
10. **Is a derived value stored beside its inputs?** `RunSummary` reads the run totals from its `outcome` rather than repeating them.
11. **Is there exactly one way to build this value?** `Pending::calls(mode)` is the only reading of a response's tool calls, so the loop can't classify one differently from how the stop policy expects. `whole_ms` is the one `Duration` conversion. One `add` raises every total.
12. **Does a constant live in prose and nowhere in code?** `6b4d0d3` found `TASK_COMPLETE` and `ToolName::MAX_LEN` written in the spec and declared nowhere.

## Types and construction

13. **Count adjacent same-typed arguments.** Two adjacent strings became `Prompts` after `start(setup, system, prompt)` was found to swap silently and invert both reported prompt sizes; five adjacent `u64` counts became `TokenCounts` for the same reason (`372ca9c`, `b1eff3d`). Then ask what would catch a swap; if nothing would, that's the finding.
14. **List the ways a value can come into existence.** Constructor, `Deserialize`, `From`, a public field. Each one holds the invariant, or the invariant isn't held. `Cost` and `Rates` are checked on both paths.
15. **Prefer a private raw type with `try_from` and `deny_unknown_fields`** over a trusting derive, so a misspelt key in a hand-written script can't read as a default.
16. **If nothing reads a type back, delete `Deserialize`** rather than write an unchecked one. `RunSummary` and `FinishedRun` were the last serde path with no rule on the way in (`2a575f8`).
17. **Make the field private when that's what holds the invariant.** `UnknownReason`'s string is private because `Other("refusal")` used to typecheck, and a refusal that skipped normalisation would have completed the run and exited 0.
18. **A new variant widens what an existing branch means.** `source()` answered `None` for one situation when the summary's `if source().is_some()` was written. `MalformedInput` gave it a second, so a model that couldn't serialise against `bash`'s schema produced a run reporting a call to a tool it doesn't have, in the one attribute whose brief reads "named a tool the run didn't offer" (`f3973d9`). The fix was a predicate named for the question the caller meant, `names_an_offered_tool`. Ask what each existing match arm assumed about the set it was matching over.
19. **Reject rather than model, when the modelling costs more than the errors it deletes.** The typestate refusal above is the case (`2a575f8`).

## Tests

`6b4d0d3` is the richest commit here.

20. **Does the test assert both halves of its name?** `a_refused_completion_leaves_the_prompt_and_the_attempts...` never checked the prompt: it cleared the input, took the refusal, then reassigned the input by hand, so clearing it on the error path would have passed.
21. **Is the state the test builds reachable through the public API?** That one's wasn't.
22. **Does a shared helper pin the value under test?** Every tally test built its status through a helper pinning `ToolSource::Builtin`, so narrowing the branch to `Builtin` alone would have dropped every MCP tool into `tool_calls_unknown` with every test green.
23. **Are two tables zipped positionally?** The stop-class table was zipped to the stop-reason table, so reordering either silently relabelled every class.
24. **Does a contract test pin all the fields or a sample of them?** The summary's JSON test pinned 8 of 21, and every field is a wide-event attribute.
25. **Is a test asserting only `is_err()` where the variant matters?** `c75290c` deleted one.
26. **Does a doc claim a mechanical property that isn't mechanical?** `spellings.rs` claimed each match is exhaustive; it held for two of the four, and the others were array literals, so a new `ToolSource` variant would have compiled, passed conformance, and emitted a value the registry has no member for.
27. **For arithmetic and aggregation, is there a law rather than an example?** Additivity, monotonicity, associativity and commutativity, idempotence, round-trip. An example says what one value does; a law says what every value does (`ad28c75`). The property pass found `cost(a) + cost(b)` differing from `cost(a + b)`, which is why a run reports the rates it was priced at.
28. **Saturating arithmetic doesn't void a law, but check rather than assume.** Saturating addition stays associative because both groupings reach the same ceiling.

## External contracts

29. **Open the documentation.** The domain modelled a provider API from memory, no adapter existed to contradict it, and no gate could (`b1eff3d`). One page read found three defects, including a request shape that returns 400 on every current model.
30. **Check the vendored conventions before inventing a `lablet.*` attribute.** `gen_ai.usage.reasoning.output_tokens` was already there while lablet discarded the count.
31. **When a claim can't be verified yet, record it against the phase where it can be** (`b1eff3d`, `227f07b`).

## Measurement

Specific to a product whose output is measurement.

32. **Does the classification blame the component that can act on it?** Tool arguments that aren't JSON were classified as a malformed response and retried, which re-rolls the same prompt against the same schema, burns the retry budget, ends the run `retries_exhausted`, and reports a tool problem as provider flakiness (`372ca9c`).
33. **Report rather than withhold.** `Pricing::cost` prices a self-contradicting usage low rather than refusing: refusing is the worse failure for a tool whose job is to report what happened, and the raw counts reach the wide event beside it.
34. **Never publish a derived number without the inputs to re-derive it.** That was the rates finding, and it was against lablet's own raw-data rule.

## Names and timing

35. **Rank names by whether the name asserts something false**, not by whether a better one exists. `6ea164d` ranked nine that way, changed four, and declined `StopClass` and `ModelRef` because neither lies and vocabulary churn has its own cost.
36. **Structural change is priced by the number of dependants.** Zero is the moment. Five commits carry the argument, in the form "before the loop is written against them."
37. **The counter-rule: don't rush a change whose real risk lives at a boundary that doesn't exist yet.** The `RunSummary` totals grouping moved to phase 4 deliberately, to land with the wide-event mapping that shows which groups it wants (`928dd0b`).

## Comments

`a72ab5f` swept both crates: 48 cuts proposed, 27 approved by a guard told to refuse anything stating a reason, a trade-off, a contract, an invariant, or an error condition.

38. **What goes:** restatement of the line below, signpost sentences announcing an argument the next two sentences make, glosses on self-naming parameters, and explanations of what Rust does rather than what lablet does.
39. **What stays:** those five, and anything that's the only record of a fact.
40. **The test:** would a reader lose a fact if the line went?

## Module organisation

From the two independent reviews behind `d59dfd1`.

41. **One module per idea, not one per stage of a run.** Name modules on one axis. `lablet-model` used two, so anything fitting neither landed in the nearest stage-module and `provider.rs` and `outcome.rs` became catch-alls.
42. **Read the file's doc comment, then its declaration order.** If the doc says "X and Y" and the declarations run X, Z, Y, then Z is the intruder. `Cost` and `Rates` sat between the two halves of `provider.rs`, pushing the request parameters below them.
43. **A module doc that needs "and" twice is two modules.**
44. **Put a `pub(crate)` helper where its only caller is.** The output cap moved to `tool.rs` because a cap is a property of a tool call and its only caller was there. A helper with callers in several modules belongs to the crate root, which is where `whole_ms` went.
45. **Read the `use` block as a claim about dependencies.** `use crate::outcome::whole_ms` in `tool.rs` implied that tools depend on outcomes.
46. **Import cycles between sibling modules are legal and are the usual sign of a boundary in the wrong place.** The fix is usually to move the shared type down into a leaf module rather than to merge the two: `ProviderKind` left `provider.rs` because `message` needed it and `provider` needs `ContentBlock` either way.
47. **A delegating wrapper duplicates the tests, not just the signature.** `Pricing::new` re-declared `Rates::new`'s signature and its `# Errors` section only to call it, and its four rate-validation tests were duplicates of the model's. Also ask whether the wrapper has a caller outside its own tests.

## Reviewing the review

48. **Does the finding name a cause or only a symptom?** Two module reviews ran without contact and reached the same shortlist; the second named the cause the first only described, and the cause became the rule that prevents recurrence (`d59dfd1`). A symptom gets fixed once.
49. **Expect a low survival rate.** 21 findings to 4 in `c75290c`, 12 to 7 in `2a575f8`, 9 names to 4 in `6ea164d`, 48 comment cuts to 27 in `a72ab5f`. A review that lands most of what it proposes probably wasn't adversarial.
50. **Give a sweep explicit refusal criteria**, not a general instruction to be careful. The comment sweep's five criteria are reusable as written.
51. **Record refutations, not just fixes.** A rejected finding recorded with its reasoning pays for itself within days.
52. **Record a deferral against the phase where it bites**, not as a vague "later" (`227f07b`, `b970009`).
53. **Record a decline with a reason that stays valid.** Splitting `message.rs` was declined because `Message<'a>` borrows from the owned types, so a module line would make the lifetime harder to follow. Splitting the large test files was declined as navigability alone, which sets the bar.
54. **Re-measure the floors after pure movement.** A move that drops a test file, or leaves a `mod tests;` pointing at nothing, shows up there and nowhere else.
55. **Does the headline match the riskiest hunk?** `d59dfd1` is labelled pure movement and contains an API change and four deleted tests. The body says so; the summary line is what a reader trusts later.

## What the gates can't see

Ask of each finding which gate should have caught it, and whether one is worth adding. These got through:

- **Mutation testing doesn't generate every narrowing.** Restricting the per-tool branch to `Builtin` is a change no mutant proposes (item 22).
- **No gate can contradict a provider API modelled from memory.** Only the documentation, or an adapter that doesn't exist yet (`b1eff3d`).
- **Coverage and mutation say nothing about placement.** Both floors were at 100% while two modules were catch-alls and two pairs imported each other.
- **A doc comment claiming a mechanical property isn't itself mechanical** until something fails to compile (item 26).
