# Unreleased (0.6.3)

0.6.3 is a security release. It fixes the findings of a security scan of the
runtime, the standard library and the installers.

## Authentication rules cover every handler they name

- **Handlers under the auth prefix.** `require_auth()` and protected-path rules now
  apply to application handlers registered under the auth route prefix (`/auth` by
  default). Before, everything under the prefix was public, so a custom
  `/auth/profile` route ran without a session. Only std/auth's own handlers there
  stay public: the login page, provider start and callback, logout, health and
  `auth_me`.
- **The path the router serves.** Auth now matches requests the way the router
  does, so `//admin`, `/admin//settings` and `/admin/` hit the rules for `/admin`
  instead of bypassing them. Protection patterns and `route_prefix` are stored in
  the same canonical form, and redirects built from the request path use it too: a
  request for `//evil.example/x` can no longer produce a protocol-relative
  `Location`.
- **Workers starting or reloading.** Protection rules no longer disappear while
  other workers start, restart or reload. Each worker re-runs the application's
  setup, and its `enable_auth()` used to clear the shared rules while worker 0 was
  already serving, so a path protected only by a rule was briefly public. While a
  server is serving, `enable_auth()` now adds rules instead of replacing them.

## Application code cannot change the runtime's security policy

The process-execution policy (`NTNT_PROCESS_ENABLE`, `NTNT_PROCESS_ALLOW`) and the
`fetch()` network policy (`NTNT_SSRF_PROTECTION`, `NTNT_ALLOW_LOCALHOST`,
`NTNT_ALLOW_PRIVATE_IPS`, `NTNT_BLOCKED_HOSTS`) are read once, from the environment
that starts `ntnt`, before any application code runs. `load_env` skips those keys,
`NTNT_NET_ALLOW_PRIVATE`, `NTNT_NETMON_ENABLE` and every `NTNT_SECRETS_*` key,
printing one warning per key and loading the rest of the file. Loading
`NTNT_ENV=development` can no longer lift a production host's localhost block;
loading `NTNT_ENV=production` still adds it.

## Cloud metadata endpoints stay blocked

`fetch()`, `download()`, `cache_fetch()` and `probe_fetch()` compare hosts in
canonical form (case, trailing dots, and IP literals in any notation), so spellings
such as `metadata.google.internal.` or `[::ffff:169.254.169.254]` are blocked.
Metadata addresses returned by DNS are denied on every request and redirect hop,
even with `NTNT_ALLOW_PRIVATE_IPS` or `NTNT_ALLOW_LOCALHOST` set; only
`NTNT_SSRF_PROTECTION=false` in the host environment turns the check off.
`NTNT_BLOCKED_HOSTS` entries are canonicalized the same way, and an IP entry also
blocks names that resolve to it.

## Safe Markdown drops unsafe links

`to_html_safe()` renders a link whose scheme is not `http`, `https` or `mailto` as
its plain text, omits an image with such a source (keeping its alt text), and no
longer renders heading attributes such as `# Title {#id .class}`. Scheme detection
follows browsers: it ignores case, leading control characters and embedded tabs or
newlines. Relative links, fragments and `to_html()` are unchanged.

## Verified TLS for PostgreSQL and Redis

- **PostgreSQL** (`std/db/postgres` and the std/auth store) honours `sslmode`:
  `verify-full` checks the certificate chain and host name, `verify-ca` the chain
  only (it needs `sslrootcert`), `require` encrypts without verifying, and `disable`
  stays plaintext. `prefer` and `allow` are rejected because they can fall back to
  plaintext. Without `sslmode`, a connection uses verified TLS (`verify-full`
  against the host CA store) unless every host is local: a loopback address,
  `localhost`, a Unix socket, or a single-label name such as a Docker service
  (`db`), which stay plaintext as before. `sslrootcert=/path/ca.pem` adds a private
  CA and always means verified TLS: without `sslmode` it selects `verify-full` for
  local hosts too, it cannot be combined with `require` or `disable`, and over a
  Unix socket, which PostgreSQL serves in plaintext only, it is an error. A TLS
  connection never falls back to plaintext, and errors never include the
  connection string. Each TLS policy has its own connection pool, and trust roots
  are read once per pool: restart after rotating a CA.
- **Redis and Valkey** (`std/kv`, `std/jobs` stores and the std/auth session store):
  `rediss://` and `valkeys://` connect over TLS and verify the certificate chain and
  host name against the host CA store. `redis://` and `valkey://` stay plaintext.
  Before, `std/kv` treated a `rediss://` URL as a SQLite file path, and jobs and auth
  rejected it.

## SMTP fixes in `std/email`

`std/email` now uses `lettre` 0.11.24, which fixes six advisories in its SMTP
client:

- [RUSTSEC-2026-0560](https://rustsec.org/advisories/RUSTSEC-2026-0560): STARTTLS response injection let a
  man-in-the-middle forge SMTP replies. Only STARTTLS connections are affected.
- [RUSTSEC-2026-0561](https://rustsec.org/advisories/RUSTSEC-2026-0561): message content with a bare LF or CR
  was not dot-stuffed, allowing SMTP command injection.
- [RUSTSEC-2026-0563](https://rustsec.org/advisories/RUSTSEC-2026-0563): mail for an address with a quoted local
  part could be sent to a different address.
- [RUSTSEC-2026-0565](https://rustsec.org/advisories/RUSTSEC-2026-0565): a malicious server could exhaust memory
  with an unbounded reply.
- [RUSTSEC-2026-0562](https://rustsec.org/advisories/RUSTSEC-2026-0562) and
  [RUSTSEC-2026-0564](https://rustsec.org/advisories/RUSTSEC-2026-0564): panics on an `EHLO` reply line of only
  white space and on a display name containing CR or LF.

## Other fixes

- The warning for JavaScript-style `${...}` interpolation in string literals scans
  each literal once and reports at most 64 candidates, shortening long snippets. A
  crafted literal previously took quadratic time and memory.
- **Windows installer:** `install.ps1` verifies the archive against the release's
  `.sha256` file before extracting it, and replaces an existing `ntnt.exe` only after
  the new binary runs and reports the release's version; on any failure the previous
  install stays. It no longer builds from source when the download fails, never
  reuses or overwrites an existing `.\ntnt` directory, and fetches the starter kit
  from the release tag instead of `main`.
- Process tests keep their fixtures in private temporary directories.

## Upgrade notes

1. If your app sets `NTNT_PROCESS_ENABLE`, `NTNT_PROCESS_ALLOW` or the `fetch()`
   policy keys in a `.env` file that it loads with `load_env`, set them in the
   environment that starts `ntnt` instead.
2. A custom handler under the auth prefix that `require_auth()` or a protected-path
   rule covers now requires a session. With a global `require_auth()`, a custom page
   mounted at the prefix itself now redirects to the prefix; exclude it in your own
   middleware.
3. `to_html_safe()` renders `tel:`, `ftp:`, `data:` and other non-allowlisted links
   as plain text. Use `to_html()` for trusted content that needs them.
4. During development hot reload, a `require_auth(path)` rule or `protected_paths`
   entry you delete stays enforced until the server restarts; added rules still
   apply at the reload.

5. A PostgreSQL connection string whose host is not local (a name containing a dot,
   or a non-loopback address) and that has no `sslmode` now requires verified TLS.
   Enable TLS on the server, add `sslrootcert=` for a private or managed CA, or add
   `sslmode=disable` to keep plaintext on a trusted network. `sslrootcert` always
   means verified TLS, local hosts included: remove it to use `disable` or
   `require`.
6. `sslmode=prefer` and `sslmode=allow` are rejected; choose `verify-full`,
   `verify-ca`, `require` or `disable`. TLS modes over a Unix socket are rejected.

See [NTNT v0.6.2](v0.6.2.md) for the previous release.
