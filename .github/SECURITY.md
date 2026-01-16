# Security Policy

## Supported Versions

NTNT is currently experimental software. Security updates are provided for the latest version only.

| Version | Supported          |
| ------- | ------------------ |
| 0.2.x   | :white_check_mark: |
| < 0.2   | :x:                |

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, use [GitHub's private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability) feature. Go to the "Security" tab of this repository and click "Report a vulnerability."

When reporting, please include:

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

## What to Expect

- **Acknowledgment**: We will acknowledge receipt within 48 hours
- **Assessment**: We will assess the severity and impact
- **Fix timeline**: Critical issues will be prioritized; expect updates within 7 days
- **Disclosure**: We will coordinate disclosure timing with you

## Scope

Security issues in the following are in scope:

- NTNT interpreter (`ntnt run`)
- NTNT CLI commands
- Standard library modules (especially `std/http/server`, `std/db/postgres`, `std/fs`)
- Intent Studio web interface

## Out of Scope

- Vulnerabilities in user-written `.tnt` code
- Issues in development dependencies not affecting production
- Theoretical attacks requiring physical access

## Recognition

We appreciate security researchers who help keep NTNT safe. Contributors who report valid security issues will be acknowledged in our release notes (unless they prefer to remain anonymous).
