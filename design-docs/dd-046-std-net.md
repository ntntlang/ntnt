# DD-046: `std/net` — Network Operations Standard Library

**Status:** Draft
**Author:** Larri
**Created:** 2026-03-22

---

## Summary

Add a `std/net` module to ntnt providing network diagnostic, monitoring, and connectivity functions. These are the building blocks for network monitoring systems, health checks, infrastructure automation, and connectivity testing — implemented in pure Rust with no external binary dependencies.

---

## Motivation

ntnt already handles HTTP (`std/http`), databases (`std/db`), and key-value stores (`std/kv`). But network operations below the HTTP layer — ping, DNS resolution, TCP connectivity, TLS inspection, port scanning — require dropping to shell commands or external tools. A developer building a network monitoring dashboard, uptime checker, or infrastructure tool has no stdlib support.

Josh's background is network engineering (MSP/ISP). The target audience for this module is exactly his domain: people building tools to monitor routers, switches, servers, and services. `std/net` gives ntnt a native advantage for this use case.

### Use Cases

- **Uptime monitoring:** Ping hosts, check TCP ports, verify HTTP endpoints
- **Network diagnostics:** DNS lookups, traceroute, TLS certificate inspection
- **Infrastructure dashboards:** Port scans, host discovery, latency graphing
- **Alerting systems:** "Ping X every 60s, alert if loss > 10%"
- **Security auditing:** TLS cert expiry, open port detection, DNS record verification
- **Automation:** SSH command execution, SNMP polling, WHOIS lookups

---

## Design Principles

1. **Pure Rust** — no shelling out to `ping`, `dig`, `nmap`, etc. All functions use Rust crates or raw socket APIs. This means ntnt binaries work on any platform without installing system tools.

2. **Consistent with existing stdlib** — returns `Result<T, String>` like other modules. Maps for structured data. Options where appropriate.

3. **Progressive disclosure** — simple functions for common cases (`ping("8.8.8.8")`), options maps for advanced config (`ping("8.8.8.8", map { "count": 10, "timeout": 5000 })`).

4. **Non-blocking where possible** — long-running operations (port scans, traceroutes) should work with ntnt's async bridge if called from the HTTP server path.

---

## Module: `std/net`

### Phase 1: Core Connectivity (ship first)

These are the most-needed functions and have the simplest Rust implementations.

#### `tcp_connect(host, port, opts?) -> Result<Map, String>`

Test TCP connectivity to a host:port with optional timeout.

```ntnt
import { tcp_connect } from "std/net"

let result = tcp_connect("db.internal", 5432)
// => Ok({ connected: true, latency_ms: 2.3, local_addr: "10.0.1.5:48230", remote_addr: "10.0.1.10:5432" })

let result = tcp_connect("offline.host", 80, map { "timeout": 2000 })
// => Err("Connection timed out after 2000ms")
```

**Rust:** `std::net::TcpStream::connect_timeout()`. Measure `Instant::now()` to `Instant::elapsed()` for latency. Zero dependencies.

**Implementation:**
- [ ] Basic connect with timeout (default 5000ms)
- [ ] Return latency_ms, local_addr, remote_addr
- [ ] `opts.timeout` override (milliseconds)
- [ ] Error messages include host:port for diagnostics

---

#### `dns_lookup(domain, record_type?) -> Result<Array<Map>, String>`

Resolve DNS records for a domain.

```ntnt
import { dns_lookup, dns_reverse } from "std/net"

let records = dns_lookup("google.com")
// => Ok([{ type: "A", value: "142.250.80.46", ttl: 300 }])

let records = dns_lookup("google.com", "MX")
// => Ok([{ type: "MX", value: "smtp.google.com", priority: 10, ttl: 3600 }])

let records = dns_lookup("example.com", "TXT")
// => Ok([{ type: "TXT", value: "v=spf1 include:_spf.google.com ~all", ttl: 300 }])

// Supported record types: A, AAAA, MX, CNAME, TXT, NS, SOA, PTR, SRV
```

**Rust:** `hickory-resolver` (formerly trust-dns). Well-maintained, pure Rust, async-capable.

**Implementation:**
- [ ] Default to A records when no type specified
- [ ] Support: A, AAAA, MX, CNAME, TXT, NS, SOA, PTR, SRV
- [ ] Return array of maps with type, value, ttl (and priority for MX/SRV)
- [ ] Custom nameserver support: `dns_lookup("example.com", "A", map { "server": "8.8.8.8" })`

---

#### `dns_reverse(ip) -> Result<String, String>`

Reverse DNS lookup (PTR record).

```ntnt
let hostname = dns_reverse("8.8.8.8")
// => Ok("dns.google")
```

**Rust:** Same `hickory-resolver`, PTR query.

- [ ] IPv4 and IPv6 support
- [ ] Return first PTR result as string

---

#### `ping(host, opts?) -> Result<Map, String>`

ICMP ping with statistics.

```ntnt
import { ping } from "std/net"

let result = ping("8.8.8.8")
// => Ok({ host: "8.8.8.8", alive: true, min: 12.3, avg: 15.1, max: 22.7, loss: 0.0, count: 4 })

let result = ping("192.168.1.1", map { "count": 10, "timeout": 3000, "interval": 200 })
// => Ok({ host: "192.168.1.1", alive: true, min: 0.5, avg: 1.2, max: 3.1, loss: 0.0, count: 10 })

let result = ping("dead.host.example")
// => Ok({ host: "dead.host.example", alive: false, loss: 100.0, count: 4 })
```

**Rust:** `surge-ping` crate. Pure Rust ICMP. **Requires `CAP_NET_RAW` capability or root.**

**Docker note:** Add `cap_add: [NET_RAW]` to docker-compose, or run the container with `--cap-add=NET_RAW`. Without it, ping returns a permissions error.

**Implementation:**
- [ ] Default: 4 pings, 1000ms timeout, 1000ms interval
- [ ] `opts.count`, `opts.timeout`, `opts.interval` overrides
- [ ] Return min/avg/max/loss statistics
- [ ] Resolve hostname to IP before pinging
- [ ] Clear error message when CAP_NET_RAW is missing
- [ ] IPv4 and IPv6 support

---

#### `tls_info(host, port?) -> Result<Map, String>`

Inspect TLS certificate and connection details.

```ntnt
import { tls_info } from "std/net"

let cert = tls_info("example.com")
// => Ok({
//   subject: "*.example.com",
//   issuer: "DigiCert Inc",
//   not_before: "2025-01-15T00:00:00Z",
//   not_after: "2026-12-01T23:59:59Z",
//   days_left: 254,
//   serial: "0A:1B:2C:...",
//   san: ["*.example.com", "example.com"],
//   protocol: "TLSv1.3",
//   cipher: "TLS_AES_256_GCM_SHA384",
//   valid: true
// })

let cert = tls_info("internal.server", 8443)
// Check non-standard ports
```

**Rust:** `rustls` + `webpki` (already in dependency tree via `reqwest`). Connect, extract cert chain, parse X.509.

**Implementation:**
- [ ] Default port 443
- [ ] Extract subject, issuer, validity dates, SAN list
- [ ] Calculate `days_left` from current time
- [ ] Extract protocol version and cipher suite
- [ ] `valid` field: cert chain validates against system roots
- [ ] Support custom port
- [ ] Timeout support (default 5000ms)

---

#### `port_scan(host, ports, opts?) -> Result<Array<Map>, String>`

Scan TCP ports on a host.

```ntnt
import { port_scan } from "std/net"

let results = port_scan("192.168.1.1", [22, 80, 443, 3306, 5432, 8080])
// => Ok([
//   { port: 22, open: true, latency_ms: 1.2 },
//   { port: 80, open: true, latency_ms: 0.8 },
//   { port: 443, open: true, latency_ms: 0.9 },
//   { port: 3306, open: false },
//   { port: 5432, open: false },
//   { port: 8080, open: true, latency_ms: 1.1 }
// ])

// Common port ranges
let results = port_scan("server.example", 1..1024, map { "timeout": 1000, "concurrency": 50 })
```

**Rust:** `std::net::TcpStream::connect_timeout()` with thread pool. No special permissions needed.

**Implementation:**
- [ ] Accept array of ports or range
- [ ] Default timeout 2000ms per port
- [ ] `opts.timeout`, `opts.concurrency` (default 20 parallel connections)
- [ ] Return array of maps with port, open, latency_ms
- [ ] Sort results by port number

---

#### `whois(domain) -> Result<String, String>`

WHOIS lookup.

```ntnt
import { whois } from "std/net"

let info = whois("example.com")
// => Ok("Domain Name: EXAMPLE.COM\nRegistrar: ...")
```

**Rust:** TCP connection to port 43 of the appropriate WHOIS server. Parse referral chain for TLDs.

**Implementation:**
- [ ] Determine WHOIS server from TLD (hardcoded map for common TLDs + IANA referral)
- [ ] Follow referrals (e.g., .com → verisign → registrar)
- [ ] Return raw WHOIS text
- [ ] Timeout support (default 10000ms)

---

#### `ip_parse(ip_or_cidr) -> Result<Map, String>`

Parse and inspect IP addresses and CIDR notation.

```ntnt
import { ip_parse, subnet_contains } from "std/net"

let info = ip_parse("192.168.1.0/24")
// => Ok({ ip: "192.168.1.0", prefix: 24, network: "192.168.1.0", broadcast: "192.168.1.255",
//         first_host: "192.168.1.1", last_host: "192.168.1.254", host_count: 254, version: 4 })

let contains = subnet_contains("10.0.0.0/8", "10.50.100.200")
// => Ok(true)
```

**Rust:** `ipnetwork` crate or `std::net::IpAddr` with manual CIDR math.

**Implementation:**
- [ ] Parse IPv4 and IPv6 addresses and CIDR notation
- [ ] Calculate network, broadcast, first/last host, host count
- [ ] `subnet_contains(cidr, ip)` for membership testing
- [ ] Validate addresses (return Err for invalid input)

---

### Phase 2: Diagnostics (after Phase 1 ships)

#### `traceroute(host, opts?) -> Result<Array<Map>, String>`

Trace network path to a host.

```ntnt
import { traceroute } from "std/net"

let hops = traceroute("8.8.8.8")
// => Ok([
//   { hop: 1, ip: "192.168.1.1", hostname: "router.local", latency_ms: 1.2 },
//   { hop: 2, ip: "10.0.0.1", hostname: "isp-gw.example.net", latency_ms: 5.3 },
//   { hop: 3, ip: "*", hostname: "*", latency_ms: null },
//   ...
//   { hop: 9, ip: "8.8.8.8", hostname: "dns.google", latency_ms: 15.1 }
// ])
```

**Rust:** `surge-ping` with incrementing TTL (ICMP approach) or TCP SYN with TTL (TCP approach). **Requires CAP_NET_RAW.**

**Implementation:**
- [ ] Default: max 30 hops, 3 probes per hop, 5000ms timeout
- [ ] `opts.max_hops`, `opts.timeout`, `opts.probes`
- [ ] Reverse DNS on each hop IP
- [ ] `*` for non-responding hops
- [ ] TCP traceroute mode (port 80/443) as alternative to ICMP

---

#### `ssh_exec(host, opts) -> Result<Map, String>`

Execute a command on a remote host via SSH.

```ntnt
import { ssh_exec } from "std/net"

let result = ssh_exec("router.internal", map {
    "user": "admin",
    "key": get_env("SSH_PRIVATE_KEY_PATH"),
    "cmd": "show ip route"
})
// => Ok({ stdout: "...", stderr: "", exit_code: 0, latency_ms: 45.2 })
```

**Rust:** `russh` (pure Rust SSH2) or `ssh2` (libssh2 bindings). `russh` is preferable — no C dependency.

**Implementation:**
- [ ] Key-based auth (path to private key)
- [ ] Password auth (optional)
- [ ] Command execution with stdout/stderr capture
- [ ] Timeout support
- [ ] Known hosts verification (optional, default permissive for monitoring use case)
- [ ] Connection reuse/pooling for repeated commands (future enhancement)

---

#### `snmp_get(host, community, oid) -> Result<Map, String>`

SNMP GET/WALK for network device monitoring.

```ntnt
import { snmp_get, snmp_walk } from "std/net"

let result = snmp_get("switch.internal", "public", "1.3.6.1.2.1.1.1.0")
// => Ok({ oid: "1.3.6.1.2.1.1.1.0", type: "OctetString", value: "Cisco IOS..." })

let interfaces = snmp_walk("switch.internal", "public", "1.3.6.1.2.1.2.2.1")
// => Ok([{ oid: "...1", type: "Integer", value: 1 }, ...])
```

**Rust:** UDP packets with ASN.1 encoding. Can use `snmp-parser` for parsing or implement the simple SNMP v2c protocol directly (it's straightforward — UDP packet with BER-encoded PDU).

**Implementation:**
- [ ] SNMP v2c GET and WALK
- [ ] Community string auth
- [ ] Timeout and retry support
- [ ] Parse common value types (Integer, OctetString, Counter32, Gauge32, TimeTicks, IPAddress)
- [ ] SNMP v3 (future — more complex, USM auth)

---

### Phase 3: Advanced (future/private module candidates)

These are more specialized and might live in a private `netmon` module rather than `std/net`:

| Function | Description | Complexity |
|---|---|---|
| `sflow_collector(port)` | Receive and parse sFlow packets | Medium — UDP listener + sFlow v5 parsing |
| `netflow_collector(port)` | Receive NetFlow/IPFIX | Medium — similar to sFlow |
| `bandwidth_test(host, opts)` | iperf-style throughput measurement | Medium — TCP stream + timing |
| `arp_scan(interface)` | ARP discovery on local subnet | Needs CAP_NET_RAW |
| `wake_on_lan(mac, broadcast?)` | Send WOL magic packet | Simple — UDP broadcast |
| `interface_list()` | List network interfaces with stats | Platform-specific |
| `route_table()` | Read system routing table | Linux: netlink, macOS: sysctl |
| `bgp_peer(host, asn)` | BGP session monitoring | Complex — full BGP FSM |

---

## Dependencies

New Cargo.toml dependencies for Phase 1:

```toml
# std/net — network operations
hickory-resolver = "0.24"          # DNS resolution (pure Rust, formerly trust-dns)
surge-ping = "0.8"                  # ICMP ping (pure Rust, needs CAP_NET_RAW)
ipnetwork = "0.20"                  # IP/CIDR parsing and math
```

For Phase 2:
```toml
russh = "0.45"                      # SSH2 (pure Rust, no C deps)
```

`tokio` is already a dependency (for axum). `hickory-resolver` integrates with tokio for async DNS. `surge-ping` also uses tokio.

**No new C dependencies.** Everything is pure Rust.

---

## File Structure

```
src/stdlib/
├── net.rs              # Phase 1: tcp_connect, dns_lookup, dns_reverse, ping,
│                       #           tls_info, port_scan, whois, ip_parse, subnet_contains
├── net_ssh.rs          # Phase 2: ssh_exec (separate file due to size/complexity)
├── net_snmp.rs         # Phase 2: snmp_get, snmp_walk
└── mod.rs              # Module registry (add "net" entry)
```

---

## Implementation Order

| PR | Functions | Effort | Dependencies |
|----|-----------|--------|-------------|
| 1 | `tcp_connect`, `ip_parse`, `subnet_contains` | Small (2-3h) | None (std::net only) |
| 2 | `dns_lookup`, `dns_reverse` | Small (2-3h) | `hickory-resolver` |
| 3 | `tls_info` | Medium (3-4h) | `rustls` (already in tree) |
| 4 | `ping` | Medium (3-4h) | `surge-ping` |
| 5 | `port_scan` | Small (2-3h) | None (std::net + thread pool) |
| 6 | `whois` | Small (1-2h) | None (raw TCP) |
| 7 | `traceroute` | Medium (4-5h) | `surge-ping` |
| 8 | `ssh_exec` | Medium (4-5h) | `russh` |
| 9 | `snmp_get`, `snmp_walk` | Medium (4-5h) | Direct UDP + ASN.1 |

**Phase 1 total:** ~15-20 hours across 6 PRs.

---

## Docker / Permissions

For functions requiring raw sockets (ping, traceroute):

```yaml
services:
  app:
    cap_add:
      - NET_RAW    # Required for ICMP ping and traceroute
```

Or at runtime: `docker run --cap-add=NET_RAW ...`

Functions that DON'T need special permissions: tcp_connect, dns_lookup, tls_info, port_scan, whois, ip_parse, ssh_exec, snmp_get.

---

## Testing Strategy

- **Unit tests:** Parse/format functions (ip_parse, subnet_contains), WHOIS server lookup
- **Integration tests:** tcp_connect to localhost, dns_lookup for known domains, tls_info for public sites
- **Mock tests:** Ping/traceroute with mock ICMP (or skip if CAP_NET_RAW unavailable)
- **CI note:** GitHub Actions runners don't have CAP_NET_RAW. Ping/traceroute tests should detect this and skip gracefully.

---

## Example: Network Monitor in ntnt

```ntnt
import { ping, dns_lookup, tcp_connect, tls_info, port_scan } from "std/net"
import { configure_queue, enqueue_in } from "std/jobs"
import { json } from "std/http/server"
import { now } from "std/time"

configure_queue(map { "store": "redis://redis:6379" })

let targets = [
    map { "name": "Web Server", "host": "web.example.com", "checks": ["ping", "http", "tls"] },
    map { "name": "Database", "host": "db.internal", "port": 5432, "checks": ["ping", "tcp"] },
    map { "name": "DNS", "host": "8.8.8.8", "checks": ["ping", "dns"] }
]

job HealthCheck on monitoring (retry: 2, timeout: 30) {
    perform(target) {
        let results = map {}

        if contains(target["checks"], "ping") {
            results["ping"] = ping(target["host"])
        }
        if contains(target["checks"], "tcp") {
            results["tcp"] = tcp_connect(target["host"], target["port"] ?? 80)
        }
        if contains(target["checks"], "tls") {
            let cert = tls_info(target["host"])
            results["tls"] = cert
            if unwrap(cert)["days_left"] < 30 {
                // Alert: cert expiring soon
                enqueue("AlertCertExpiry", map { "host": target["host"], "days": unwrap(cert)["days_left"] })
            }
        }
        if contains(target["checks"], "dns") {
            results["dns"] = dns_lookup(target["host"], "A")
        }

        // Store results for dashboard
        kv_set(kv, "health:#{target["name"]}:#{now()}", results)

        // Re-enqueue for next check
        enqueue_in("HealthCheck", 60, target)
    }
}

// Kick off monitoring
for target in targets {
    enqueue("HealthCheck", target)
}

// Dashboard endpoint
get("/api/health", fn(req) {
    // ... query recent health check results from KV ...
    return json(results)
})

work_async(map { "concurrency": 4 })
listen(8080)
```

This is a complete network monitoring system in ~50 lines of ntnt.
