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

Design documents are maintained in place. When a decision changes, rewrite the
document so it still describes current truth; Git and the decision log preserve
the chronology. Do not create a new document merely because a new work item
revisited the same question.

## `decision-log.md`: chronology

[`decision-log.md`](decision-log.md) records decisions newest first. Each row
states what changed, why, and which current document owns the resulting rule.
It does not repeat implementation details or become a second architecture
description.

## Single source of truth

Give every settled claim one canonical home. A top-level document may summarize
a design rule at the depth needed for its mental model, then link to the design
document that owns the precise contract and rationale. Prefer stable module and
type names over line numbers or exhaustive lists of participants.

