# Fetch redirects in 0.5.4

`fetch(url)`, `fetch(options)`, `fetch(url, options)`, cache misses, and both
download forms follow redirects by default. This changes the 0.5.3 default.

To retain a terminal redirect response, pass `map { "redirect": "manual" }`.
Use this explicitly for first-hop status monitors, redirect inspection, and
security-header checks that must describe the requested origin rather than a
redirect destination.
For a URL-only call, use `fetch(url, options)`; for a download, put `url` and
`redirect` in its request map. Request-map downloads retain their safer file
defaults: explicitly set `overwrite` and `create_parent` in the third argument
when migrating code that depended on the legacy string download's file options.

| Option | Behavior |
| --- | --- |
| `redirect: "follow"` (default) | Follow 301/302/303/307/308 with per-hop checks |
| `redirect: "manual"` | Return actual response; downloads reject non-success status |
| `redirect: "error"` | Reject redirect status, even without a valid Location |
| `follow_redirects: true / false` | Alias for follow / manual |
| `max_redirects: 5` | Followed-edge limit, 1..10; does not override the mode |

Conflicting aliases and invalid options fail before contact or cache lookup.
Default/follow/true share cache identity; manual/false share another. Mode and
hop limit separate cache entries. Secret-bearing requests still cannot be cached.

Default and error modes use one 30-second timeout budget unless configured and
cap response bytes at `NTNT_MAX_RESPONSE_SIZE` (50 MiB default). Text's decoded
UTF-8 has a separate cap of the same size. Charset, BOM and malformed-sequence
replacement remain compatible with manual text decoding. Explicit manual keeps
legacy timeout, size and download cancellation behavior. Synchronous DNS and
filesystem operations may delay observing the deadline or cancellation.

Default/error reject explicit Host, URL userinfo and control characters. Following
also rejects HTTPS downgrades, cycles, invalid Location and protected destinations.
DNS validation still checks every returned address and pins that exact set;
existing SSRF grants are unchanged. Follow returns a terminal response when
Location is absent. Fetch's final HTTP 4xx/5xx remain `Ok` with `ok: false`.

Same-origin HTTPS redirects preserve plaintext and Secret credentials and replayable
bodies. Same-origin development loopback HTTP uses the existing Secret transport
exception and independent SSRF gate. Across origins, credentials and caller headers
are permanently removed except non-Secret Accept, Accept-Language and User-Agent.
Returning to the original origin never restores them. There is no redirect cookie jar.

POST on 301/302 and non-GET/HEAD on 303 become GET with body sources and entity
headers removed. GET-with-body on 303 retains its body. Other bodies, including
Secret JSON/form leaves, may replay only on the same origin; cross-origin replay
fails before contacting the destination.

Redirect and request diagnostics do not expose supplied URL queries or credentials.
Download non-success errors report status only, never a reflected response body.
A denied next request does **not** undo an earlier request: the original POST may
already have been processed. Do not infer that retrying is safe.

Downloads retain destination preservation on precommit failure and atomic promotion.
If a no-overwrite download commits but temporary cleanup fails, success includes
`cleanup_warning` and `temporary_path`; it does not claim rollback.
