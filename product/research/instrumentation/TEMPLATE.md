# <Source group>

Researched: <date>. Sources are linked inline. Everything here is an inventory of what the source emits or expects; recommendations for lablet are confined to the final section and marked as opinion.

## Sources covered

One bullet per source: name, version or date checked, link to the primary document (spec, SDK docs, source file).

## Span structure

How the source models a run: span names, hierarchy (run > turn > llm call > tool call, or otherwise), what is the root, what is a child, what is a span event versus a span.

## Attributes

Table: `name | type | set on (span kind / event / resource) | required or optional | description | source(s)`. Use the exact name the source uses. Group by namespace.

## Metrics

Table: `name | instrument (counter/histogram/gauge) | unit | dimensions | description | source(s)`.

## Events and log records

Table: `name | carried on (span event / log record) | fields | when emitted | source(s)`. Include content-capture events and their opt-in mechanism.

## Per-run summary fields

What the source records once per run, sample, or trace (as opposed to per step): scores, totals, outcome flags, cost, and how they are exposed (final span attributes, a summary record, an API object).

## Execution environment

What the source captures about where the run happened: host, container, git commit, working directory, resource usage (CPU, memory, wall time), tool versions, dependency versions, seeds, sandbox identifiers.

## Notable design choices

Short bullets: anything about naming, cardinality, content redaction, sampling, cost computation, or correlation ids worth copying or avoiding.

## Recommendation for lablet (opinion)

Bullets, each: item, adopt / adapt / skip, one-line reason. Keep to the ten most important.
