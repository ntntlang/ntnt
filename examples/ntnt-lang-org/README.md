# NTNT Language Website

The official website for ntnt-lang.org, built with NTNT.

## Local Development

```bash
# Without database (demo mode)
ntnt run server.tnt

# With PostgreSQL blog
createdb ntnt_blog
psql -d ntnt_blog -f schema.sql
DATABASE_URL=postgres://localhost/ntnt_blog ntnt run server.tnt
```

Visit http://localhost:8080

## Production Deployment (Docker + Cloudflare)

The site runs behind Cloudflare Tunnel for SSL, CDN, and DDoS protection.

### Prerequisites

1. A Cloudflare account with your domain (ntnt-lang.org)
2. Docker and Docker Compose installed on your server

### Setup Cloudflare Tunnel

Tunnels are managed in Cloudflare's **Zero Trust** section (separate from your domain settings):

1. Go to [Cloudflare Dashboard](https://dash.cloudflare.com)
2. Click **Zero Trust** in the left sidebar (or go to [one.dash.cloudflare.com](https://one.dash.cloudflare.com))
3. Navigate to **Networks** → **Connectors**
4. Select the **Cloudflare Tunnels** tab
5. Click **Add a tunnel**
6. Select **Cloudflared** as connector type
7. Name your tunnel (e.g., `ntnt-lang-org`)
8. Copy the tunnel token from the install command
9. Add a public hostname:
   - **Subdomain**: Leave blank for root domain, or enter `www`
   - **Domain**: Select `ntnt-lang.org` from dropdown
   - **Service Type**: `HTTP`
   - **URL**: `ntnt:8080` (matches Docker service name)

### Deploy

```bash
cd examples/ntnt-lang-org

# Create .env with your tunnel token
cp .env.example .env
# Edit .env and add your CLOUDFLARE_TUNNEL_TOKEN

# Build and run
docker-compose up -d

# View logs
docker-compose logs -f

# Stop
docker-compose down
```

### Architecture

```
Internet → Cloudflare Edge (SSL, CDN, Analytics) → cloudflared tunnel → NTNT server
```

- **SSL/TLS**: Handled by Cloudflare (automatic certificates)
- **Analytics**: Available in Cloudflare dashboard
- **DDoS Protection**: Included with Cloudflare
- **Performance**: NTNT's async server handles 100k+ req/sec

## Structure

```
ntnt-lang-org/
├── server.tnt           # Main server entry point
├── website.intent       # Intent file for IDD
├── schema.sql           # PostgreSQL schema for blog
├── assets/              # Static files (logos, images)
├── middleware/
│   ├── logger.tnt       # Request logging
│   └── layout.tnt       # HTML layout wrapper
└── routes/
    ├── index.tnt        # Homepage
    ├── learn.tnt        # Learn/documentation page
    └── blog.tnt         # Blog list and post pages
```

## Features

- **Homepage**: Introduction, why NTNT, key features, CTA to GitHub
- **Learn**: Quick start, IDD tutorial, CLI reference, documentation links
- **Blog**: PostgreSQL-backed with markdown rendering (works in demo mode without DB)

## Colors

From the NTNT logo:

- Teal: #137EA2
- Orange: #F1993F
- Indigo: #2C2A74
- Navy: #192F52
- Yellow: #FCDD83
- Cyan: #20DDE3
