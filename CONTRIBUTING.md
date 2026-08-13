# Contributing

Thanks for helping make OffPDF better.

OffPDF is intentionally local-first. Contributions should preserve the core
promise: user files stay on the user's machine.

## Before Opening A Pull Request

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
npm run build
npm test
```

## Engineering Guidelines

- Prefer existing UI and command patterns.
- Prefer file paths for core operations. If a preview or editor needs document
  data in the interface, keep it local and limit it to the smallest page-level
  payload required.
- Spawn local tools with argument arrays, not shell strings.
- Keep temporary files in the app temp area and clean them up after jobs.
- Make errors actionable for non-technical users.

## Pull Request Checklist

- The app still works offline.
- User files are not uploaded or logged.
- New dependencies have compatible licenses.
- `npm run build` passes.
- `npm test` passes.
- Documentation is updated when needed.
