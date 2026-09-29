# Documentation Policy

Everything under `docs/` is written for developers changing Revia. Product-user
guidance belongs in the repository README or dedicated user documentation, not
here.

This file defines how the directory is organized. It deliberately does not list
the documents that happen to exist; adding or removing a topic must not require
editing this policy.

## Top level: the mental model

[`architecture.md`](architecture.md) is the entry point. Each other top-level
document explains one coherent architectural unit: its purpose, important
concepts, ownership boundaries, dependencies, and governing invariants. Every
top-level document must be reachable from `architecture.md`.

Top-level documents are compressed mental models, not file catalogs or complete
specifications. A rule belongs here when a developer needs it to predict where
behavior and state belong. Exact procedures, edge cases, canonical forms, and
the defense of one choice over alternatives belong under `design/`.

These documents describe established behavior and accepted architectural
direction. Temporary gaps, planned migrations, and unresolved proposals belong
in the work tracker rather than in the architectural model.

## `design/`: one design question per file

Each file under `design/` owns one question that could reasonably have been
answered another way. It records the current rule, the reason for that rule,
rejected alternatives, and only the history needed to understand the current
choice. Worked examples and edge cases belong here when they are needed to
implement or verify the rule.

These are not ADRs. When a decision changes, rewrite its document in place so
it continues to describe the current rule and rationale rather than becoming a
dated record. The decision log preserves the superseded decision and why Revia
revised it. Do not create a new document merely because a new work item
revisited the same question.

## `decision-log.md`: design decision history

[`decision-log.md`](decision-log.md) preserves design decisions whose historical
context would otherwise disappear when a design document is rewritten in
place. A row records that Revia chose or revised a durable design rule among
meaningful alternatives, summarizes why, and links to the document that owns
the current rule and rationale.

The log does not record implementation or documentation activity. Implementing,
completing, testing, or refactoring an existing decision does not add a row;
neither does synchronizing documentation with code or summarizing a change or
release. If the design rule and its rationale did not change, the log does not
change.

## Single source of truth

Give every settled claim one canonical home. A top-level document may summarize
a design rule at the depth needed for its mental model, then link to the design
document that owns the precise contract and rationale. Prefer stable module and
type names over line numbers or exhaustive lists of participants.
