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
- `ntnt test` sends its requests once the app's server is listening, instead of
  after a fixed short wait, so an app that is slow to start no longer fails its
  first requests; `--port 0` now picks a free port.
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

See [NTNT v0.6.2](v0.6.2.md) for the previous release.
