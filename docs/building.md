# Building and packaging

You need Windows 10 (1809 or later) or Windows 11, x64, with Rust, a current
Node.js (23.6 or later), the Visual Studio C++ Build Tools (desktop workload and
Windows SDK) and WebView2. Builds are tested on Windows 11. The Rust version is
pinned in `rust-toolchain.toml`, and the lockfiles are checked in.

Packaging the bundled X server also requires Python 3.14 or later.

To build the app:

```powershell
npm ci
npm run desktop:build
```

The executable ends up in `target/release/yuuterm-desktop.exe`.

To run the same checks as CI and create the release packages:

```powershell
npm ci
npm run build
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features yuuterm-desktop/capture -- -D warnings
cargo test --workspace
npm run test:ui
npm run test:downloads
npm run test:archives
pwsh -NoProfile -File scripts/Build-Release.ps1
```

`npm run build` has to come before clippy and the tests: Tauri embeds the built
UI (`dist/`) when the Rust code compiles.

`Build-Release.ps1` puts the installer, the portable ZIP, the source of the
bundled X server and `SHA256SUMS.txt` into `target/distribution`. Publish all of
them together; see [RELEASING.md](../RELEASING.md).

X11 downloads use Windows `curl.exe` with connection and transfer deadlines,
bounded retries and SHA-512 verification. Each package is logged; a stalled mirror
fails with the package name instead of silently consuming the entire CI job.
Archive listing and extraction also have deadlines. Both output streams are
captured, owner/permission restoration is disabled, and archive links are resolved
as regular files after extraction. CI tests the actual small Bash package before
the expensive Rust checks and release build.

The GitHub Actions workflow runs these steps on every push to `main` and uploads
the packages as build artifacts. It never publishes a release by itself.

The RSA advisory exception (see [rsa-advisory-review.md](rsa-advisory-review.md))
is limited to a reviewed dependency set and a review date. The dependency check
fails when either changes; review it instead of disabling the check. Some
hardware and remote-server tests are opt-in and don't run in CI.
