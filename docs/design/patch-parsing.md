# Patch Parsing Boundary

## Question

Should parsed patch syntax be the canonical diff model or a format-specific
intermediate representation?

## Decision

Parsed patch syntax is an intermediate representation between captured input
and the semantic diff. Patch dialect is recognized per file entry, not once for
an entire stream. Plain unified entries and Git entries remain distinct; Git
entries further distinguish two-way, combined, binary, and bodyless forms.

Parsed `@@` hunks preserve the ranges and rows declared by the input, but they
are neither semantic changes nor durable identity. Combined entries own two or
more ordered parent sides. Every combined hunk and parent-indexed metadata value
must have the same arity, so ranges or prefixes cannot be associated with the
wrong parent.

The parser retains syntax needed to identify files, interpret changes, or show
source rows. Unknown metadata that cannot affect recognized boundaries is
skipped; malformed structure and unsupported body forms are errors. There is no
`Opaque` semantic node because the exact capture already preserves unused text.

## Why

Plain unified and Git extended syntax make different claims, while neither is a
sufficient semantic model. Git carries rename, mode, binary, gitlink, and
combined-parent facts outside ordinary hunks. Preserving those distinctions at
the parsing boundary lets semantic interpretation use them without coupling the
rest of the application to patch headers.

## Rejected alternatives

- One stream-wide `Unified | Git` tag cannot describe an empty or mixed stream.
- Treating combined hunks as ordinary hunks invents one before side where the
  input supplied several.
- Treating binary and bodyless entries alike loses whether content changed.
- Retaining every unknown line as an `Opaque` value duplicates the exact capture
  without giving the value review meaning.
