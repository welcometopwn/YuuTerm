# Releasing

1. Set the version in `Cargo.toml`, `package.json` and
   `crates/yuuterm-desktop/tauri.conf.json`, run `npm install` and `cargo check`,
   then merge the change into `main` through a pull request.
2. Tag it: `git tag v<version>` and `git push origin v<version>`. GitHub Actions
   builds and tests the tag and creates a draft release with all files.
3. Rewrite the draft text under `### Security`, `### Added`, `### Changed`,
   `### Fixed` or `### Removed`, only those that apply, one short bullet per
   change. Keep the compare link and the footer. Publish.

For an existing tag, run the workflow manually (Actions → Windows preview → Run
workflow) with the tag; delete an existing release for that tag first. Always
publish the X11 source ZIP with the binaries: the bundled Cygwin/X runtime is
LGPL and its source has to be available next to them.
