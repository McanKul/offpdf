# Code signing policy

OffPDF publishes release artifacts only from the protected `main` branch through
the GitHub Actions workflows stored in this repository. A source commit, release
tag, build log, and resulting artifact must remain traceable to one another.

## Current platform policy

- Apple Silicon macOS releases are signed with an Apple Developer ID certificate,
  notarized by Apple, and verified before publication.
- Windows x64 installers may be published only through GitHub Releases while the
  Authenticode workflow is being prepared. They must be clearly marked as
  unsigned, include a SHA-256 file, and pass the documented build, install,
  startup, runtime, and Microsoft Defender checks before attachment.
- Unsigned Windows installers are not linked from offpdf.com. Once Authenticode
  is enabled, the application executable, installer, and uninstaller must pass
  signature and timestamp verification before the website links the package.

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
3. Each platform workflow completes its documented verification and smoke-test
   gates before it can attach a release artifact. macOS additionally requires a
   valid signature and notarization ticket.
4. The temporary unsigned Windows path verifies the unsigned state, produces a
   SHA-256 file, installs and launches the app, checks bundled runtimes, and runs
   Microsoft Defender. The future signed path must additionally verify the
   application, installer, uninstaller, and RFC 3161 timestamp.
5. A failed or unverifiable artifact is discarded rather than published.

OffPDF has no automatic updater. Users choose when to download and install a
release. See the [privacy statement](./PRIVACY.md), the
[uninstall instructions](https://offpdf.com/uninstall), and the
[security policy](./SECURITY.md) for related project policies.
