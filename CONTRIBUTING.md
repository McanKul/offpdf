# Contributing

Thanks for helping make OffPDF better.

OffPDF is intentionally local-first. Contributions should preserve the core
promise: user files stay on the user's machine.

## Branch and pull request workflow

`development` is the integration branch for ongoing work. `main` contains the
latest release-ready code.

1. Fork the repository and create your branch from `development`.
2. Keep the change focused and add or update tests where practical.
3. Open the pull request against `McanKul/offpdf:development`, not `main`.
4. Link the related issue and explain how you verified the change.

For a larger feature or behavior change, open an issue first so the scope can be
agreed before significant work begins.

## Before opening a pull request

- Keep changes focused and easy to review.
- Update docs when behavior, setup, packaging, or privacy expectations change.
- Do not commit build artifacts, downloaded engine binaries, signing
  certificates, secrets, or local config.
- Avoid adding network features, telemetry, analytics, or cloud dependencies.
  If a change needs network access, open an issue first and explain the user
  benefit and privacy impact.
- Review licenses before adding PDF engines or bundling third-party binaries.

## Local Setup

```bash
npm install
npm run tauri:dev
```

Useful checks:

```bash
npm run check:versions
npm run build
npm test
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

`npm run check:versions` verifies that the release version matches in
`package.json`, both root version fields in `package-lock.json`,
`src-tauri/Cargo.toml`, the OffPDF entry in `src-tauri/Cargo.lock`, and
`src-tauri/tauri.conf.json`. Update these together when changing the version.
CI runs this check on pull requests and reports mismatched files and values.

## Engineering guidelines

- Prefer existing UI and command patterns.
- Prefer file paths for core operations. If a preview or editor needs document
  data in the interface, keep it local and limit it to the smallest page-level
  payload required.
- Spawn local tools with argument arrays, not shell strings.
- Keep temporary files in the app temp area and clean them up after jobs.
- Make errors actionable for non-technical users.

## Pull request checklist

- The app still works offline.
- User files are not uploaded or logged.
- New dependencies have compatible licenses.
- `npm run build` passes.
- `npm test` passes.
- `cargo check --manifest-path src-tauri/Cargo.toml` passes for Rust changes.
- `cargo test --manifest-path src-tauri/Cargo.toml` passes for Rust changes.
- Documentation is updated when needed.
