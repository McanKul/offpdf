# Security Policy

OffPDF handles local documents, so security and privacy reports are taken
seriously.

## Supported versions

Security fixes target the latest published release and the current default
branch.

## Reporting a vulnerability

Please do not post exploit details or sensitive sample documents in a public
issue. Email **kul3562@gmail.com** with the subject `OffPDF security report`.

A useful report includes:

- A short description of the issue and its potential impact.
- The smallest set of steps needed to reproduce it.
- The operating system and OffPDF version or commit.
- A minimal synthetic sample file, if one is necessary and safe to share.

Do not send a private or confidential document. A redacted or newly generated
fixture is strongly preferred.

## Scope

Security-sensitive areas include:

- File handling and path validation.
- Subprocess invocation.
- Temporary file cleanup.
- Packaged third-party binaries.
- Dependency vulnerabilities.
- Anything that could break the offline, no-upload privacy promise.
