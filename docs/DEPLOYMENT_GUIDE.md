# Deployment Guide for NTNT Applications

This guide covers deploying NTNT applications to production — web servers, background job workers, databases, and scaling.

## Architecture Overview

A typical ntnt production deployment:

```
Internet → Cloudflare Edge (SSL, CDN, DDoS)
              ↓
           cloudflared tunnel
              ↓
     ┌────────────────────┐
     │   ntnt web server   │  ← HTTP requests, enqueues jobs
     │   (server.tnt)      │
     └────────┬───────────┘
              │ shared KV store (Redis/SQLite)
     ┌────────┴───────────┐
     │   ntnt workers      │  ← processes background jobs
     │   (ntnt worker)     │
     └────────────────────┘
              │
     ┌────────┴───────────┐
     │   PostgreSQL/SQLite │  ← application data
     └────────────────────┘
```

**Key principle:** Web servers and workers are separate processes that share a job queue (Redis or SQLite). Scale them independently based on request volume vs. job throughput.

---

## Quick Start: Docker + Cloudflare Tunnel

The recommended deployment uses Docker with Cloudflare Tunnel:

- **Zero exposed ports** — cloudflared connects outbound, no firewall rules needed
- **Automatic SSL** — Cloudflare handles certificates
- **Built-in CDN** — static assets cached at edge
- **DDoS protection** — included free

### Dockerfile

ntnt apps use prebuilt base images with the ntnt binary included:

```dockerfile
FROM ntnt:0.4.6

WORKDIR /app

# Copy application files
COPY server.tnt .
COPY routes ./routes
COPY views ./views
COPY assets ./assets
COPY lib ./lib
COPY jobs ./jobs

# If using jobs directory
# COPY jobs ./jobs

RUN chown -R ntnt:ntnt /app
USER ntnt

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/ || exit 1

CMD ["ntnt", "run", "server.tnt"]
```

### docker-compose.yml (Web Only)

```yaml
services:
  app:
    build: .
    container_name: my-ntnt-app
    restart: unless-stopped
    environment:
      - NTNT_TIMEOUT=${NTNT_TIMEOUT:-30}
      - DATABASE_URL=${DATABASE_URL:-}
    networks:
      - app-network
    # No ports exposed — only accessible via tunnel

  cloudflared:
    image: cloudflare/cloudflared:latest
    container_name: cloudflared
    restart: unless-stopped
    command: tunnel run
    environment:
      - TUNNEL_TOKEN=${CLOUDFLARE_TUNNEL_TOKEN}
    networks:
      - app-network
    depends_on:
      app:
        condition: service_healthy

networks:
  app-network:
    driver: bridge
```

---

## Background Jobs

ntnt includes a built-in background job system with priority queues, retry logic, and multiple storage backends (Redis, SQLite, Valkey).

### Defining Jobs

```ntnt
import { configure_queue, enqueue } from "std/jobs"

configure_queue(map { "store": "redis://redis:6379" })

job ProcessOrder on orders (retry: 3, timeout: 120) {
    perform(order_id) {
        let order = fetch("#{API_BASE}/orders/#{order_id}")
        // ... process order ...
    }
    on_failure(error, attempt) {
        notify_slack("Order failed: #{error} (attempt #{attempt})")
    }
}
```

### Running Workers

There are three ways to run job workers:

**1. Inline with web server** — simplest, good for small apps:
```ntnt
// server.tnt
work_async(map { "concurrency": 4 })  // background worker threads
listen(8080)                            // HTTP server
```

**2. CLI worker** — same source file, separate process:
```bash
ntnt worker server.tnt --concurrency 10
ntnt worker server.tnt --concurrency 5 --queues emails,payments
```

**3. Dedicated worker file** — separate concerns entirely:
```ntnt
// worker.tnt — no HTTP, just jobs
import "./lib/helpers.tnt"
import "./lib/jobs.tnt"

configure_queue(map { "store": "redis://redis:6379" })
work_jobs(map { "concurrency": 10 })  // blocks until Ctrl-C
```

### Job Directory Auto-Discovery

For apps with many jobs, use directory-based organization:

```ntnt
// server.tnt
jobs("jobs/")   // discovers all .tnt files in jobs/ directory
listen(8080)
```

```
my-app/
├── server.tnt
├── lib/
│   └── notifications.tnt
└── jobs/
    ├── send_email.tnt
    ├── process_order.tnt
    └── generate_report.tnt
```

Each job file is evaluated in the shared interpreter context. Job declarations are registered automatically. In dev mode, modified job files are detected and reloaded on the next request cycle.

---

## Scaling: Separate Web and Worker Processes

For production workloads, run web servers and workers as independent processes. This lets you scale each based on its actual load.

### Pattern 1: Same File, Different Entry Points

The simplest approach — one source file, two ways to run it:

```yaml
# docker-compose.yml
services:
  web:
    build: .
    command: ntnt run server.tnt
    # Handles HTTP requests, enqueues jobs

  worker:
    build: .
    command: ntnt worker server.tnt --concurrency 10
    # Processes jobs, no HTTP server

  worker-emails:
    build: .
    command: ntnt worker server.tnt --concurrency 5 --queues emails
    # Dedicated email queue worker
```

`ntnt worker` evaluates the source file in Worker mode — `listen()`, `work_async()`, and `serve_static()` are automatically suppressed. Only job definitions, imports, and configuration run.

### Pattern 2: Separate Files

For larger apps where web and worker concerns diverge:

```ntnt
// server.tnt — web only
import "./lib/helpers.tnt"
jobs("jobs/")

configure_queue(map { "store": "redis://redis:6379" })
routes("routes/")
listen(8080)
```

```ntnt
// worker.tnt — jobs only
import "./lib/helpers.tnt"
jobs("jobs/")

configure_queue(map { "store": "redis://redis:6379" })
work_jobs(map { "concurrency": 10 })
```

```yaml
services:
  web:
    build: .
    command: ntnt run server.tnt

  worker:
    build: .
    command: ntnt run worker.tnt
    deploy:
      replicas: 3   # 3 worker instances × 10 concurrency = 30 parallel jobs
```

### Scaling Guidelines

| Component | Scale by | Typical ratio |
|-----------|----------|---------------|
| Web servers | Request volume | 1-5 instances |
| Workers | Queue depth / job duration | 1-10+ instances |
| Redis | Usually single instance | 1 (with replication for HA) |
| PostgreSQL | Query volume | 1 (with read replicas if needed) |

**Worker concurrency:** Each worker thread processes one job at a time. Set `--concurrency` based on job characteristics:
- **CPU-bound jobs** (image processing, computation): concurrency = CPU cores
- **I/O-bound jobs** (API calls, email): concurrency = 10-50
- **Mixed:** start at 10, adjust based on throughput metrics

---

## Database Configuration

### PostgreSQL (Recommended for Production)

```yaml
services:
  db:
    image: postgres:16
    restart: unless-stopped
    environment:
      POSTGRES_DB: myapp
      POSTGRES_USER: ntnt
      POSTGRES_PASSWORD: ${DB_PASSWORD}
    volumes:
      - pg-data:/var/lib/postgresql/data
    networks:
      - app-network

  app:
    environment:
      - DATABASE_URL=postgres://ntnt:${DB_PASSWORD}@db:5432/myapp

volumes:
  pg-data:
```

**Connection pooling:** ntnt uses deadpool-postgres with configurable pool size:
```bash
NTNT_DB_POOL_SIZE=5  # per-worker pool (total = workers × databases × pool_size)
```

### SQLite (Simpler, Single-Server)

No separate service needed — SQLite runs embedded:

```ntnt
import { connect } from "std/db/sqlite"
let db = unwrap(connect("app.db"))
```

For jobs with SQLite backend:
```ntnt
configure_queue(map { "store": "sqlite:./jobs.db" })
```

**Note:** SQLite works well for single-server deployments. For multi-server (separate web + worker containers), use Redis for the job queue so both processes can access it.

### Redis (Job Queue Backend)

```yaml
services:
  redis:
    image: redis:7-alpine
    restart: unless-stopped
    volumes:
      - redis-data:/data
    networks:
      - app-network

volumes:
  redis-data:
```

---

## Cloudflare Tunnel Setup

### Create a Tunnel

1. Go to [Cloudflare Zero Trust](https://one.dash.cloudflare.com)
2. Navigate to **Networks** → **Connectors** → **Cloudflare Tunnels**
3. Click **Add a tunnel** → **Cloudflared** → name it
4. Copy the tunnel token from the install command

### Configure Public Hostname

In the tunnel configuration:
1. **Public Hostname** tab → **Add a public hostname**
2. Domain: select your domain
3. Service Type: `HTTP`
4. URL: `app:8080` (Docker service name + port)

### SSL/TLS Settings

In your domain dashboard:
- **SSL/TLS** → Encryption mode: **Full**
- **SSL/TLS** → **Edge Certificates** → Enable **Always Use HTTPS**

### Environment File

```bash
# .env (add to .gitignore!)
CLOUDFLARE_TUNNEL_TOKEN=your-token-here
DB_PASSWORD=your-db-password
NTNT_TIMEOUT=30
```

---

## Full Production Example

A complete deployment with web server, workers, PostgreSQL, and Redis:

```yaml
services:
  # Web server — handles HTTP, enqueues jobs
  web:
    build: .
    command: ntnt run server.tnt
    restart: unless-stopped
    environment:
      - DATABASE_URL=postgres://ntnt:${DB_PASSWORD}@db:5432/myapp
      - REDIS_URL=redis://redis:6379
      - NTNT_ENV=production
    depends_on:
      db:
        condition: service_healthy
      redis:
        condition: service_started
    networks:
      - app-network

  # Background workers — process jobs, no HTTP
  worker:
    build: .
    command: ntnt worker server.tnt --concurrency 10
    restart: unless-stopped
    environment:
      - DATABASE_URL=postgres://ntnt:${DB_PASSWORD}@db:5432/myapp
      - REDIS_URL=redis://redis:6379
      - NTNT_ENV=production
    depends_on:
      db:
        condition: service_healthy
      redis:
        condition: service_started
    networks:
      - app-network

  # Cloudflare tunnel — only routes to web, not workers
  cloudflared:
    image: cloudflare/cloudflared:latest
    command: tunnel run
    restart: unless-stopped
    environment:
      - TUNNEL_TOKEN=${CLOUDFLARE_TUNNEL_TOKEN}
    depends_on:
      web:
        condition: service_healthy
    networks:
      - app-network

  # PostgreSQL
  db:
    image: postgres:16
    restart: unless-stopped
    environment:
      POSTGRES_DB: myapp
      POSTGRES_USER: ntnt
      POSTGRES_PASSWORD: ${DB_PASSWORD}
    volumes:
      - pg-data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U ntnt"]
      interval: 5s
      timeout: 3s
      retries: 5
    networks:
      - app-network

  # Redis (job queue)
  redis:
    image: redis:7-alpine
    restart: unless-stopped
    command: redis-server --appendonly yes
    volumes:
      - redis-data:/data
    networks:
      - app-network

networks:
  app-network:
    driver: bridge

volumes:
  pg-data:
  redis-data:
```

---

## Alternative: Systemd (No Docker)

For single-server deployments without Docker:

### Install ntnt

```bash
git clone https://github.com/ntntlang/ntnt
cd ntnt
cargo build --release
sudo cp target/release/ntnt /usr/local/bin/
```

### Web Server Service

Create `/etc/systemd/system/ntnt-web.service`:
```ini
[Unit]
Description=NTNT Web Server
After=network.target postgresql.service redis.service

[Service]
User=www-data
WorkingDirectory=/var/www/myapp
ExecStart=/usr/local/bin/ntnt run server.tnt
Environment=NTNT_ENV=production
Environment=DATABASE_URL=postgres://ntnt:pass@localhost/myapp
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

### Worker Service

Create `/etc/systemd/system/ntnt-worker.service`:
```ini
[Unit]
Description=NTNT Job Workers
After=network.target postgresql.service redis.service

[Service]
User=www-data
WorkingDirectory=/var/www/myapp
ExecStart=/usr/local/bin/ntnt worker server.tnt --concurrency 10
Environment=NTNT_ENV=production
Environment=DATABASE_URL=postgres://ntnt:pass@localhost/myapp
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

Enable and start:
```bash
sudo systemctl daemon-reload
sudo systemctl enable ntnt-web ntnt-worker
sudo systemctl start ntnt-web ntnt-worker
```

---

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `NTNT_ENV` | `development` | `production` disables hot-reload, detailed errors |
| `NTNT_TIMEOUT` | `30` | Request timeout in seconds |
| `NTNT_MAX_BODY_SIZE` | `10MB` | Maximum request body size |
| `NTNT_SECURITY_HEADERS` | `true` | Auto security headers (HSTS, X-Frame-Options, etc.) |
| `NTNT_SSRF_PROTECTION` | `true` (prod) | Block SSRF in `fetch()` |
| `NTNT_DB_POOL_SIZE` | `5` | Database connections per pool per worker |
| `NTNT_DETAILED_ERRORS` | `true` (dev) | Show stack traces in error responses |

---

## Hot-Reload (Development Only)

In development (`NTNT_ENV=development`, the default), ntnt watches for file changes:

- **Source files** — main `.tnt` file and imported modules are reloaded on change
- **Route files** — `routes/` directory is rescanned for new/changed/deleted routes
- **Job files** — `jobs/` directory is rescanned for new/changed job definitions
- **Templates** — `views/` templates are reloaded via mtime-based cache invalidation

Hot-reload is disabled in production (`NTNT_ENV=production`).

**Job hot-reload limitations:** Changed perform block logic takes effect on the next job run. New imports or helper functions require a server restart (workers cache their interpreter at startup). Deleted job files leave ghost definitions until restart (harmless — never enqueued).

---

## Troubleshooting

### Tunnel shows "Unhealthy"
- Check container logs: `docker compose logs cloudflared`
- Verify tunnel token in `.env`
- Ensure web container is running and healthy

### 502 Bad Gateway
- Check app logs: `docker compose logs web`
- Verify hostname URL matches Docker service name (`web:8080`)
- Test health endpoint: `docker compose exec web curl localhost:8080/`

### Jobs not processing
- Verify workers are running: `docker compose logs worker`
- Check job status: `ntnt jobs status server.tnt`
- Verify web and workers connect to the same Redis: check `REDIS_URL`
- Check for dead jobs: `ntnt jobs list server.tnt --status=dead`

### Workers crashing on startup
- Check for import errors: `docker compose logs worker`
- Verify the source file evaluates cleanly: `ntnt run server.tnt` (should start without errors)
- Worker mode suppresses `listen()` but all imports and job definitions must be valid

---

## Production Checklist

- [ ] Environment variables configured (`.env` file, in `.gitignore`)
- [ ] `NTNT_ENV=production` set
- [ ] Health check endpoint responding
- [ ] SSL/TLS set to "Full" in Cloudflare
- [ ] "Always Use HTTPS" enabled
- [ ] Database backups configured
- [ ] Redis persistence enabled (`appendonly yes`)
- [ ] Container restart policy set (`restart: unless-stopped`)
- [ ] Worker concurrency tuned for job characteristics
- [ ] Job retry limits and timeouts configured
- [ ] Dead job monitoring in place (`ntnt jobs list --status=dead`)
- [ ] Logs accessible and monitored
- [ ] `robots.txt` and `sitemap.xml` deployed
