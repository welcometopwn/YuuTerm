# Releasing

Replace `<version>` with the new version, for example `0.3.0`.

1. Set the version in the root `Cargo.toml`, `package.json` and
   `crates/yuuterm-desktop/tauri.conf.json`, then run `npm install` and
   `cargo check` so the lockfiles follow.
2. Add a `## <version>` section at the top of [CHANGELOG.md](CHANGELOG.md).
3. Commit, push to `main` and wait for the GitHub Actions build to pass.
4. Tag the release:

   ```powershell
   git tag v<version>
   git push origin v<version>
   ```

   The tag starts the `Windows preview` workflow. It builds and tests the tagged
   source, then creates a draft release with the installer, portable ZIP, X11
   source ZIP and `SHA256SUMS.txt`. The release text is the version's section
   from CHANGELOG.md. Versions below 1.0 and versions with a suffix such as
   `-beta.1` are marked as pre-releases.
5. Check the draft on GitHub, then publish it.

To build a release for a tag that already exists, run the workflow manually
(Actions → Windows preview → Run workflow) and enter the tag, for example
`v0.2.1`. If a release for the tag already exists, delete it first.

Release assets always come from the workflow, not from a local build. Always
publish the X11 source ZIP with the binaries: the bundled Cygwin/X runtime
is LGPL and its source has to be available next to them. `SHA256SUMS.txt` lets
people verify downloads; it is not a signature. Say in the release text that the
files are unsigned.
