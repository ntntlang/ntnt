# Security Policy

## Supported Versions

NTNT is currently experimental software. Security updates are provided for the latest version only.

| Version | Supported          |
| ------- | ------------------ |
| 0.2.x   | :white_check_mark: |
| < 0.2   | :x:                |

## Dependency monitoring

Rust dependencies are checked against the [RustSec advisory database](https://rustsec.org/).

- **Pull requests and pushes to `main`:** the read-only `RustSec audit` job checks the committed `Cargo.lock` on every run, including documentation-only PRs. A known vulnerability fails that job and the existing `CI Status` summary. Fork PRs need no write token or repository secrets. Informational warnings are printed but are not silently promoted to verified application vulnerabilities.
- **Daily:** `Scheduled dependency audit` runs at 13:17 UTC (GitHub may delay scheduled runs). It opens deduplicated public issues for **already-published upstream advisories**, including maintenance/soundness notices. It also preserves the audit's failing exit status for known vulnerabilities. This exception does not permit public disclosure of a newly discovered NTNT vulnerability; use the private reporting procedure below for those.
- **Manual:** run `Scheduled dependency audit` with **Run workflow** to audit a selected ref. Manual runs do not open advisory issues.
- **Dependabot:** `.github/dependabot.yml` configures Cargo security-update PRs, with routine version-update PRs disabled via `open-pull-requests-limit: 0`. This does **not** enable GitHub's repository-level security settings or auto-merge fixes.

### One-time administrator setup

After merging the monitoring configuration, an administrator must confirm that the dependency graph, **Dependabot alerts**, and **Dependabot security updates** are enabled in this repository's security settings. See [GitHub's configuration instructions](https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/secure-your-dependencies/configure-security-updates). Repository write permission alone cannot verify or change those settings.

Retain `CI Status` as a required check if branch protection is used. The RustSec job is included in that summary; it is not merely an advisory badge. Review and test update PRs normally; this setup never merges or deploys them automatically. Newly introduced or pre-existing vulnerabilities keep the check red until remediated or a specifically reviewed, justified exception is approved. There is no default ignore list.

### Local audit and remediation

Install the same scanner used by CI and audit the existing lockfile:

```bash
cargo install cargo-audit --version 0.22.2 --locked
cargo audit --file Cargo.lock
```

CI runs the scanner from outside the checkout and passes the lockfile explicitly, so it does not load project-local Cargo configuration or generate a missing lockfile. Scanner/action versions are pinned; updating those pins is separate maintenance from refreshing the advisory database, which happens on each audit.

Prefer the smallest compatible upgrade to a patched version and inspect the dependency path and enabled features before judging application exposure. Dependabot may not be able to resolve a transitive fix that requires upgrading its parent or changing application code. Do not suppress an advisory just to make CI green. Rebuild and release affected binaries after dependency fixes; a fixed lockfile does not update already-deployed runtimes. These checks cover known Rust crate advisories, not zero-days or operating-system/container packages and CA stores.

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
