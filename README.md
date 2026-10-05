# Revia

Revia is an interactive terminal UI for reviewing Git diffs. It opens Git
comparisons or unified patches as a navigable changeset without modifying the
working tree, the index, or commits.

## Inputs

```text
revia git changes
revia git staged
revia git unstaged
revia git revision <REVISION>
revia git range <A..B|A...B>
revia patch <FILE|->
```

Running `revia` without a source is equivalent to `revia git changes`.
`--print` writes the captured patch without opening the TUI.

## 0.1.0 controls

- `j` / `k`, arrows, page keys, `g` / `G`: move through the diff
- `[` / `]`, `,` / `.`: move by hunk or file
- `/`, then `n` / `N`: search and move through matches
- `1` / `2` / `0`: split, stack, or responsive layout
- `s`, `Tab`: show and focus the file rail
- `w`, `m`, `-` / `=`: wrapping, headers, and diff context
- `r`: reload the selected input
- `?`: complete keyboard help
- `q`: quit
