# Web Deployment for NTNT Applications

This guide covers deploying NTNT web applications to production.

## Recommended: Docker + Cloudflare Tunnel

The simplest and most secure deployment uses Docker with Cloudflare Tunnel:

```
Internet → Cloudflare Edge (SSL, CDN, DDoS) → cloudflared tunnel → NTNT server
```

**Benefits:**
- **Zero exposed ports** - Your server has no public-facing ports
- **Automatic SSL** - Cloudflare handles certificates
- **Built-in CDN** - Static assets cached at edge
- **DDoS protection** - Included free
- **Analytics** - Traffic stats in Cloudflare dashboard
- **Simple setup** - No nginx, no certbot, no firewall rules

---

## Step 1: Create Your Dockerfile

Create a `Dockerfile` in your project:

```dockerfile
# Multi-stage build for smaller final image
FROM rust:1.86-bookworm AS builder

WORKDIR /build
COPY . /ntnt-source
WORKDIR /ntnt-source
RUN cargo build --release

# Runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 ntnt

WORKDIR /app

# Copy binary
COPY --from=builder /ntnt-source/target/release/ntnt /usr/local/bin/ntnt

# Copy your application files
COPY server.tnt .
COPY routes ./routes
COPY views ./views
COPY assets ./assets

RUN chown -R ntnt:ntnt /app
USER ntnt

EXPOSE 8080

# Health check for Docker and Cloudflare
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/ || exit 1

ENV NTNT_TIMEOUT=30

CMD ["ntnt", "run", "server.tnt"]
```

## Step 2: Create docker-compose.yml

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
    # No ports exposed - only accessible via tunnel

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

## Step 3: Set Up Cloudflare Tunnel

### 3.1 Add Your Domain to Cloudflare

If you haven't already:

1. Sign up at [cloudflare.com](https://cloudflare.com)
2. Click **Add a site** and enter your domain
3. Select the **Free** plan (sufficient for most sites)
4. Update your domain's nameservers to the ones Cloudflare provides
5. Wait for DNS propagation (usually 5-30 minutes)

Once active, your domain dashboard will show **DNS Setup: Full** in the overview.

### 3.2 Create a Tunnel (Zero Trust)

Tunnels are managed in Cloudflare's **Zero Trust** section, separate from your domain settings:

1. Go to [Cloudflare Dashboard](https://dash.cloudflare.com)
2. Click **Zero Trust** in the left sidebar (or go to [one.dash.cloudflare.com](https://one.dash.cloudflare.com))
3. Navigate to **Networks** → **Connectors**
4. Select the **Cloudflare Tunnels** tab
5. Click **Add a tunnel**
6. Select **Cloudflared** as the connector type
7. Name your tunnel (e.g., `ntnt-lang-org`)
8. Click **Save tunnel**

### 3.3 Copy Your Tunnel Token

After creating the tunnel, you'll see connector setup instructions. Find the token in the install command:

```bash
cloudflared service install eyJhIjoiNjk...  # This long string is your token
```

Copy this entire token string - you'll need it for your `.env` file.

### 3.4 Configure Public Hostname

Still in the tunnel configuration, add a public hostname to route traffic:

1. Click the **Public Hostname** tab
2. Click **Add a public hostname**
3. Configure:
   - **Subdomain**: Leave blank for root domain (`ntnt-lang.org`), or enter `www`
   - **Domain**: Select your domain from dropdown
   - **Path**: Leave blank
   - **Service Type**: `HTTP`
   - **URL**: `ntnt:8080` (matches your Docker service name and port)
4. Click **Save hostname**

Add another hostname for `www` if desired (same settings, just add `www` as subdomain).

### 3.5 Configure Domain SSL/TLS Settings

Go back to your domain dashboard (click **Back to Domains** or select your domain):

1. **SSL/TLS** → **Overview**
   - Verify encryption mode is **Full** (should be set by default)

2. **SSL/TLS** → **Edge Certificates**
   - Enable **Always Use HTTPS**
   - Enable **Automatic HTTPS Rewrites**

3. **Security** → **Settings** (optional but recommended)
   - Enable **Bot Fight Mode** to block malicious bots
   - Enable **Browser Integrity Check**

4. **Speed** → **Optimization** (optional)
   - Enable **Auto Minify** for JavaScript, CSS, HTML
   - Enable **Brotli** compression

## Step 4: Deploy

### 4.1 Create Environment File

```bash
# Create .env file
cat > .env << 'EOF'
CLOUDFLARE_TUNNEL_TOKEN=your-token-here
NTNT_TIMEOUT=30
DATABASE_URL=postgres://user:pass@db:5432/myapp
EOF
```

**Important:** Add `.env` to your `.gitignore`:
```bash
echo ".env" >> .gitignore
```

### 4.2 Build and Run

```bash
# Build and start in background
docker-compose up -d --build

# View logs
docker-compose logs -f

# Check status
docker-compose ps
```

### 4.3 Verify Deployment

1. Check container health: `docker-compose ps`
2. Check tunnel status in Cloudflare dashboard (should show "Healthy")
3. Visit your domain - it should load with HTTPS

## Step 5: Enable Cloudflare Analytics

Analytics are automatically available in your Cloudflare dashboard:

1. Go to **Analytics & Logs** → **Traffic**
2. View:
   - Total requests and unique visitors
   - Bandwidth usage
   - Geographic distribution
   - Top paths and status codes
   - Threats blocked

For more detailed analytics, consider **Web Analytics** (free):
1. Go to **Analytics & Logs** → **Web Analytics**
2. Add a site and copy the JS snippet
3. Add to your HTML templates

---

## Optional: Cloudflare Caching

To cache static assets at Cloudflare's edge:

1. Go to **Caching** → **Cache Rules**
2. Create a rule:
   - **If**: URI Path starts with `/assets` or `/static`
   - **Then**: Cache eligible, Edge TTL: 1 month
3. Your static files are now served from Cloudflare's global CDN

---

## Optional: Rate Limiting

To protect against abuse:

1. Go to **Security** → **WAF** → **Rate limiting rules**
2. Create a rule:
   - **If**: URI Path contains `/api`
   - **Then**: Block for 10 minutes when rate exceeds 100 requests per minute

---

## Alternative: Systemd (No Docker)

For simpler deployments without Docker, you can run NTNT directly with systemd.

### Install NTNT

```bash
# Build from source
git clone https://github.com/ntntlang/ntnt
cd ntnt
cargo build --release
sudo cp target/release/ntnt /usr/local/bin/
```

### Create Systemd Service

Create `/etc/systemd/system/ntnt-app.service`:

```ini
[Unit]
Description=My NTNT Application
After=network.target

[Service]
User=www-data
Group=www-data
WorkingDirectory=/var/www/myapp
ExecStart=/usr/local/bin/ntnt run server.tnt
Environment=NTNT_TIMEOUT=30
Environment=DATABASE_URL=postgres://user:pass@localhost/db
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable ntnt-app
sudo systemctl start ntnt-app
```

### Connect to Cloudflare Tunnel

Install cloudflared on your server:

```bash
# Debian/Ubuntu
curl -L https://pkg.cloudflare.com/cloudflare-main.gpg | sudo tee /usr/share/keyrings/cloudflare-archive-keyring.gpg
echo "deb [signed-by=/usr/share/keyrings/cloudflare-archive-keyring.gpg] https://pkg.cloudflare.com/cloudflared $(lsb_release -cs) main" | sudo tee /etc/apt/sources.list.d/cloudflared.list
sudo apt update
sudo apt install cloudflared
```

Configure and run as a service:

```bash
sudo cloudflared service install <YOUR-TUNNEL-TOKEN>
```

---

## Troubleshooting

### Tunnel shows "Unhealthy"

1. Check container logs: `docker-compose logs cloudflared`
2. Verify token is correct in `.env`
3. Ensure the app container is running and healthy

### 502 Bad Gateway

1. Check app is running: `docker-compose logs app`
2. Verify the hostname URL matches your service name (`app:8080`)
3. Check health endpoint responds: `docker-compose exec app curl localhost:8080/`

### SSL Certificate Errors

1. Ensure Cloudflare SSL mode is set to "Full" (not "Flexible")
2. Wait a few minutes for certificate provisioning
3. Clear browser cache and try again

### Slow First Request

Normal - the first request after deployment may be slower as Cloudflare establishes the tunnel connection. Subsequent requests are fast.

---

## Production Checklist

- [ ] Environment variables configured (not hardcoded)
- [ ] `.env` file in `.gitignore`
- [ ] Health check endpoint working
- [ ] SSL/TLS set to "Full" or "Full (strict)"
- [ ] "Always Use HTTPS" enabled
- [ ] Caching rules configured for static assets
- [ ] Rate limiting configured for API endpoints
- [ ] Automatic container restart enabled (`restart: unless-stopped`)
- [ ] Logs accessible (`docker-compose logs`)
