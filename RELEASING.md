# Releasing

1. Open **Actions -> Version -> Run workflow**.
2. Choose `patch`, `minor`, or `major`.
3. Merge the generated release PR after `Linux`, `macOS`, and `MSRV` pass.

Merging the PR creates the version tag and starts the Release workflow. Release builds Linux and macOS archives, publishes the crate, writes checksums, and publishes the GitHub Release.

To retry a failed release, run the Release workflow with the existing tag.
