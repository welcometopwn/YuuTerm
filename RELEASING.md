# Releasing

Replace `<version>` with the new version, for example `0.3.0`.

1. Set the version in the root `Cargo.toml`, `package.json` and
   `crates/yuuterm-desktop/tauri.conf.json`, then run `npm install` and
   `cargo check` so the lockfiles follow.
2. Add a `## <version>` section at the top of [CHANGELOG.md](CHANGELOG.md).
3. Commit, push to `main` and wait for the GitHub Actions build to pass.
4. Build the packages locally (see [docs/building.md](docs/building.md)):

   ```powershell
   pwsh -NoProfile -File scripts/Build-Release.ps1
   ```

5. Tag the release:

   ```powershell
   git tag v<version>
   git push origin v<version>
   ```

6. Create a draft release with every file from `target/distribution`. Copy the
   version's section from CHANGELOG.md into the release text:

   ```powershell
   gh release create v<version> --verify-tag --draft --prerelease --title "YuuTerm <version>" target/distribution/*
   ```

   Drop `--prerelease` once a version is no longer a preview.

7. Check the draft on GitHub, then publish it.

Always upload the X11 source ZIP with the binaries: the bundled Cygwin/X runtime
is LGPL and its source has to be available next to them. `SHA256SUMS.txt` lets
people verify downloads; it is not a signature. Until code signing is active
(see [docs/code-signing.md](docs/code-signing.md)), say in the release text that
the files are unsigned.
