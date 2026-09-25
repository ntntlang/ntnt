# Monitoring start deadlines

Monitoring freshness is a **start authorization**, not a response timeout. A queued
probe must not start an obsolete measurement merely because its worker lease is
still valid.

## Contract

The monitoring APIs accept two independently optional fields:

- `start_deadline_ms`: a nonnegative absolute UTC Unix-epoch millisecond deadline.
- `start_monotonic_deadline_ms`: a nonnegative process-local deadline from
  `std/time.monotonic_deadline()` or the same `monotonic_now()` origin.

Absent or `None` means that clock imposes no restriction. Other values are
rejected, not coerced. Equality is expired. Supplying both clocks lets either
clock veto a start; a local monotonic deadline must never be persisted or copied
between processes. Distributed clock-skew policy remains the application's job.

The final check occurs at the socket-operation boundary, after request and socket
preparation. An expired authorization returns `Err("start_deadline_expired")`
without initiating that operation. Applications must treat this as a collection
gap with **no measurement timestamp**, not evidence that a target is down.

The boundary is a userspace check immediately before initiating the socket
operation. It is not a guarantee of syscall-entry, NIC transmission, or remote
receipt before a wall-clock instant: operating-system preemption, buffering and
retransmission remain outside this contract. This is stronger than checking only
before calling a high-level network primitive, which may perform blocking setup
before it ever reaches the socket.

Once the first operation may have contacted the target, it cannot be reclassified
as an unsent expiry. The operation's existing completion timeout remains in force;
a valid response finishing after the start deadline is still a valid response.
An ambiguous post-send failure is not permission to repeat the measurement.

## Protocol boundaries

- `std/http.probe_fetch`: bounded monitoring GET over a fresh, direct HTTP/1.1
  connection. The first TCP connect attempt starts the sample, before TLS and HTTP
  exchange. Redirects are returned rather than followed. There is no pooled
  connection, proxy, or automatic HTTP-request replay. At most 16 complete probes run
  concurrently process-wide; saturation fails before target contact. Ordinary `fetch`
  is unchanged.
- `std/net.dns_lookup`: the first UDP send, or a TCP connection attempt when DNS
  starts over TCP, starts resolution. Deadline-enabled resolution uses Hickory's
  normal DNS behavior through a guarded runtime provider, not a replacement DNS
  parser. Subsequent DNS protocol work belongs to that already-started resolution.
- `std/net.ping`: the first ICMP send starts the configured finite sample. Requested
  later packets and intervals remain subject to the aggregate timeout, not the
  start deadline. Persistent ping and traceroute are not changed by this option.
- `std/netmon.snmp_get`: the first request datagram starts the operation. Explicit
  bounded protocol retries remain part of that operation; applications requiring
  one measurement send should request `retries: 0`. `snmp_walk` is unchanged.

Hostname resolution needed to locate an HTTP/ICMP target is setup, not an HTTP or
ICMP sample. Resolver traffic is not covered by a promise of zero target-probe
contact. For a DNS check itself, its DNS traffic is the monitored operation.

## Application integration

Keep the durable scheduled time and deadline in the application's run record.
After credentials and authorization, derive a conservative local monotonic budget
without extending the original UTC deadline. Pass both deadlines to the runtime,
which performs the final socket guard. Persist original measurement evidence
without running the probe again when database writes need retrying.

Do not infer absence of a send from an arbitrary transport error or a lost worker.
Only the documented no-send expiry result establishes that this runtime guard
rejected the start. Queue ownership, durable evidence, cross-run fencing and
uncertain-outcome reconciliation remain separate responsibilities.
