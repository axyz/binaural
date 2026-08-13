# Releasing

Releases use tags matching `v<version>` from `Cargo.toml`. GitHub Actions builds Linux and macOS archives, publishes the crate when needed, creates checksums, then publishes the GitHub Release.

## First crates.io release

Trusted Publishing requires an existing crate. Publish `0.1.0` once with a scoped crates.io API token:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo publish --dry-run --locked
cargo login
cargo publish --locked
```

Then configure the crate's Trusted Publisher on crates.io:

- GitHub owner: `axyz`
- Repository: `binaural`
- Workflow: `release.yml`
- Environment: `release`

After one successful automated publication, enable Trusted Publishing Only on crates.io.

Tag the already-published first version to create its binary release. The workflow detects the existing crate version and skips republishing it.

## Subsequent releases

1. Update `version` in `Cargo.toml`.
2. Run `cargo check` to update the workspace package entry in `Cargo.lock`, then commit both files.
3. Merge the version change into `main` with CI green.
4. Tag that commit and push the tag:

```sh
git switch main
git pull --ff-only
git tag v0.1.1
git push origin v0.1.1
```

Failed release builds remain drafts. Re-run the workflow after fixing the failure; uploads use `--clobber`.
