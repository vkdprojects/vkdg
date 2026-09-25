# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| latest (main) | ✅ |
| pre-1.0 releases | ❌ |

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Use GitHub's private vulnerability reporting:
[Report a vulnerability](https://github.com/vkdprojects/vkdg/security/advisories/new)

Please include:

- Type and severity of the vulnerability
- Affected component (crate name and version)
- Steps to reproduce
- Potential impact

You should receive a response within **72 hours**. If the issue is confirmed we will
release a patch as soon as possible — typically within 7 days for critical issues.

## Scope

In scope: vulnerabilities in VKDG source code, including credential handling, tenant
isolation, stream commit semantics, and plugin sandbox boundaries.

Out of scope: vulnerabilities in upstream dependencies (report to the dependency
maintainer), theoretical weaknesses without a demonstrated exploit path.

## Disclosure Policy

We follow coordinated disclosure. We will credit researchers in the release notes
unless they prefer to remain anonymous.
