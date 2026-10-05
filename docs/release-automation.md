# Release Automation

Revia uses one release tag and two tools with non-overlapping ownership.

| Side effect | Owner |
| --- | --- |
| Version and changelog update | `cargo-release` |
| Release commit | `cargo-release` |
| crates.io publication | `cargo-release` |
| `v<version>` tag and push | `cargo-release` |
| Binary builds and archives | `dist` |
| SHA-256 checksums | `dist` |
| GitHub Release and artifact upload | `dist` |
| Homebrew formula generation and tap publication | `dist` |

The `Publish crate` GitHub Actions workflow is the only supported execution
path for a release. It runs `cargo-release` from the default branch and accepts
the SemVer release version as an explicit workflow input. The workflow reads
the crates.io token from the `CARGO_REGISTRY_TOKEN` Actions secret. Do not store
a registry token in repository files or pass one as a workflow input.

The workflow pushes its commit and tag with `RELEASE_GITHUB_TOKEN`. This secret
must contain a service-account personal access token with repository contents
write access because a push made with the built-in `GITHUB_TOKEN` does not
trigger the tag-driven `Release` workflow. Restrict creation of matching
release tags to this release principal: the generated workflow treats a
matching tag and its commit as trusted release input.

`cargo-release` publishes the crate before it pushes the release commit and
`v<version>` tag. The tag triggers the generated `Release` workflow, where
`dist` builds and publishes the binary artifacts and updates
`choplin/homebrew-tap`. The dist workflow does not publish to crates.io, update
versions, or create commits or tags in the Revia repository. It does create and
push the formula commit in the Homebrew tap, using a `HOMEBREW_TAP_TOKEN` with
contents write access to that tap.

The generated `.github/workflows/release.yml` is derived from
`dist-workspace.toml` by `dist` 0.33.0. Do not edit the workflow by hand; change
the dist configuration and regenerate it. The dist configuration pins every
referenced GitHub Action to an audited commit; update those pins there and
regenerate rather than changing the generated workflow. cargo-binstall reads
the metadata in `Cargo.toml` and installs the `{name}-{target}.tar.xz` archive
produced by `dist`, so it introduces no independent release side effect.

The Nix flake package is built directly from the selected repository revision;
it has no release-time publication step or project binary cache. It therefore
does not duplicate any side effect owned by `cargo-release` or `dist`.

## Validate a release

Install `cargo-release` 1.1.6 and `dist` 0.33.0, then run the package and both
planning paths from a clean checkout with Rust 1.90.0:

```sh
cargo +1.90.0 package --locked
VERSION=x.y.z
cargo +1.90.0 release --dry-run "$VERSION"
dist plan
dist generate --check
nix build .#revia
./result/bin/revia --version
```

These commands do not publish, commit, tag, push, create a GitHub Release, or
update the Homebrew tap. The cargo-release dry run validates the single
`[Unreleased]` replacement in `CHANGELOG.md`, repeats package verification, and
prints the planned release actions. The dist plan must contain only these
targets and include the Homebrew installer and publisher:

- `aarch64-apple-darwin`
- `x86_64-apple-darwin`
- `x86_64-unknown-linux-gnu`

Pull requests run the generated dist workflow in `upload` mode. They build the
same target matrix and retain the archives and SHA-256 checksums as workflow
artifacts without creating a GitHub Release or modifying the tap.

## Publish a release

1. Confirm `main` is clean and all required checks pass.
2. Confirm `CARGO_REGISTRY_TOKEN`, `RELEASE_GITHUB_TOKEN`, and
   `HOMEBREW_TAP_TOKEN` exist as Actions secrets with the permissions described
   above.
3. Run the `Publish crate` workflow from `main` with the selected version.
4. Confirm crates.io contains that `revia` version.
5. Confirm the tag-triggered `Release` workflow publishes archives and SHA-256
   checksums for exactly the three supported targets and updates the `revia`
   formula in `choplin/homebrew-tap`.
6. Install through Homebrew and cargo-binstall and verify `revia --version`.

Do not run the workflow again for an already-published version. If crates.io
publication succeeds but the release commit or tag push fails, inspect the
repository, crates.io, and remote tags to determine the last completed
`cargo-release` step. Resume only the missing commit/tag/push operation for the
same version; do not publish the crate again and do not use `dist` to recreate
any `cargo-release` side effect. The `Release` workflow can be rerun after the
tag exists because it only consumes that tag.
