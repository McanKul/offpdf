# Code signing policy

OffPDF publishes release artifacts only from the protected `main` branch through
the GitHub Actions workflows stored in this repository. A source commit, release
tag, build log, and resulting artifact must remain traceable to one another.

## Current platform policy

- Apple Silicon macOS releases are signed with an Apple Developer ID certificate,
  notarized by Apple, and verified before publication.
- Public Windows distribution is paused while the Authenticode signing workflow
  is being prepared. An unsigned Windows validation build must not be attached to
  a public GitHub Release or linked as an official download.
- Windows publication will resume only after the application executable,
  installer, and uninstaller pass Authenticode and timestamp verification.

## Roles and approval

- Contributors submit changes through pull requests against `development`.
- Repository maintainer `@McanKul` reviews release integration and is the current
  signing-policy approver.
- Signing credentials are restricted to GitHub Actions secrets or the signing
  provider. They are never committed to the repository or exposed to pull
  request workflows.
- Every production signing request requires explicit maintainer approval.

## Release controls

1. Required frontend and Rust checks must pass on the release commit.
2. Release builds run on GitHub-hosted runners from the tagged `main` commit.
3. Each platform workflow completes its explicitly documented signature,
   notarization, and smoke-test gates before it can attach a release artifact.
4. The future Windows workflow must additionally verify the application,
   installer, uninstaller, and RFC 3161 timestamp before publishing anything.
5. A failed or unverifiable artifact is discarded rather than published.

OffPDF has no automatic updater. Users choose when to download and install a
release. See the [privacy statement](./PRIVACY.md), the
[uninstall instructions](https://offpdf.com/uninstall), and the
[security policy](./SECURITY.md) for related project policies.
