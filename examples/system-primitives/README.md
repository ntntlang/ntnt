# Native system primitives (0.5.4)

Run `ntnt lint --strict examples/system-primitives` first.

- `ntnt run examples/system-primitives/files.tnt` exercises private publication and byte hashing, removes its unique local directory, and exits. Unix-only operations explicitly report unsupported on other platforms.
- `ntnt run examples/system-primitives/smtp-fixture.tnt` prints the bound loopback address, polls at most two clients for eight seconds, and closes everything. Send `DATA\r\n`, binary CRLF-delimited data, then `.\r\n`. Lines starting `..` lose one dot. Line buffers cap at 4096 bytes and captures at 8192. This is a bounded byte-framing demonstration, not a general SMTP or MIME implementation.
- `NTNT_WORKERS=1 ntnt run examples/system-primitives/http-fixture.tnt` runs until terminated. It implements a redirect, a small JSON CAS/version state endpoint, and a no-send email capture endpoint. Launchers scan `NTNT_READY {"host":...,"port":...}`, set connection deadlines, and terminate/reap the child on every exit. State belongs to one worker; this is not distributed storage.
- `ntnt intent check examples/system-primitives/system.intent` executes the pure native crypto assertions. TCP server handles cannot be used in native Intent observer mode; TCP and HTTP process fixtures are exercised by Rust integration tests through ordinary strict `ntnt run`.

`listen` remains a global builtin. Fixture options suppress selected automatic and helper-supplied response headers; authentication, CSRF, CSP, and security headers retain their ordinary behavior. HTTP 204 responses have no body. Date, reason phrases, and framing need not match a different server implementation.
