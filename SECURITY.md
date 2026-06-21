# Security Policy

OffPDF handles local documents, so security and privacy reports are taken
seriously.

## Supported Versions

OffPDF is currently pre-release. Security fixes target the latest public source
on the default branch until stable release channels exist.

## Reporting A Vulnerability

Please do not post exploit details in a public issue.

Use GitHub Security Advisories once the public repository is available. If that
is not available yet, contact the maintainers privately through the project
website or maintainer profile.

Useful reports include:

- A short description of the issue.
- Steps to reproduce.
- A minimal sample file if the issue requires one and it can be shared safely.
- The operating system and OffPDF version or commit.

## Scope

Security-sensitive areas include:

- File handling and path validation.
- Subprocess invocation.
- Temporary file cleanup.
- Packaged third-party binaries.
- Dependency vulnerabilities.
- Anything that could break the offline/no-upload privacy promise.
