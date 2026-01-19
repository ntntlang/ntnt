# Web Deployment for NTNT Applications

For a production setup, the recommended approach (similar to Node.js or Go apps) is to run the application as a **systemd service** behind **Nginx** as a reverse proxy.

This creates a robust architecture where:
*   **Nginx** handles SSL/TLS, compression, caching, and static files.
*   **Systemd** ensures the app auto-starts on boot and restarts if it crashes.
*   **NTNT** handles the application logic.

## Option 1: Systemd Service + Nginx (Recommended)

This is the most efficient way to run on a standard Linux server (Ubuntu/Debian).

### 1. Create a Systemd Service

Create a file at `/etc/systemd/system/ntnt-app.service`:

```ini
[Unit]
Description=My NTNT Application
After=network.target

[Service]
# User that runs the app (create this user for security)
User=www-data
Group=www-data

# Path to your application directory
WorkingDirectory=/var/www/myapp

# Command to start the app using the install location of ntnt
# Ensure 'ntnt' includes the full path if not in global PATH
ExecStart=/usr/local/bin/ntnt run server.tnt

# Environment variables
Environment=PORT=3000
Environment=NODE_ENV=production
Environment=DATABASE_URL=postgres://user:pass@localhost/db

# Automatically restart on crash
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

Reload the systemd daemon (required to pick up the new file), then enable and start the service:

```bash
sudo systemctl daemon-reload
sudo systemctl enable ntnt-app
sudo systemctl start ntnt-app
```

### 2. Nginx Configuration

Edit your site config (e.g., `/etc/nginx/sites-available/myapp.com`):

```nginx
server {
    listen 80;
    server_name myapp.com;

    # Optional: Serve static files directly with Nginx for performance
    # Matches the serve_static() path in your .tnt file
    location /static/ {
        alias /var/www/myapp/public/;
        expires 30d;
        access_log off;
    }

    location / {
        # Forward requests to your local ntnt instance
        proxy_pass http://127.0.0.1:3000;
        
        # Standard proxy headers
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host $host;
        proxy_cache_bypass $http_upgrade;
        
        # Pass real client IP
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

Enable the site by linking it to `sites-enabled`, then test and reload Nginx:

```bash
sudo ln -s /etc/nginx/sites-available/myapp.com /etc/nginx/sites-enabled/
sudo nginx -t
sudo systemctl reload nginx
```

### 3. Setup SSL with Let's Encrypt

The easiest way to secure your site with HTTPS (port 443) is using Certbot.

1.  **Install Certbot and the Nginx plugin:**

    ```bash
    sudo apt-get update
    sudo apt-get install certbot python3-certbot-nginx
    ```

2.  **Run Certbot:**

    ```bash
    sudo certbot --nginx -d myapp.com
    ```

    Follow the prompts. Certbot will automatically:
    *   Obtain a certificate.
    *   Modify your Nginx config to listen on port 443 (SSL).
    *   Set up automatic redirection from HTTP to HTTPS.
    *   Set up a cron job for certificate renewal.

3.  **Verify Configuration:**

    Your Nginx config (`/etc/nginx/sites-available/myapp.com`) will automatically be updated to something like this:

    ```nginx
    server {
        server_name myapp.com;
        
        # SSL Configuration (Added by Certbot)
        listen 443 ssl; 
        ssl_certificate /etc/letsencrypt/live/myapp.com/fullchain.pem; 
        ssl_certificate_key /etc/letsencrypt/live/myapp.com/privkey.pem; 
        include /etc/letsencrypt/options-ssl-nginx.conf; 
        ssl_dhparam /etc/letsencrypt/ssl-dhparams.pem;

        location /static/ {
            alias /var/www/myapp/public/;
            expires 30d;
        }

        location / {
            proxy_pass http://127.0.0.1:3000;
            # ... proxy headers ...
        }
    }

    # Redirect HTTP to HTTPS (Force SSL)
    # We strictly redirect all port 80 traffic to 443. Nothing is served on 80.
    server {
        listen 80;
        server_name myapp.com;
        return 301 https://$host$request_uri;
    }
    ```

## Option 2: Docker Setup

If you prefer containerization (e.g., for easy deployment or scaling), here is a standard `Dockerfile`.

**Dockerfile**:

```dockerfile
# Use a lightweight Linux base (Debian-based usually easiest for Rust binaries)
FROM debian:bullseye-slim

# Install dependencies needed for `ntnt` (openssl, cacerts)
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl-dev \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Install ntnt
# (Assuming you download a release binary or copy it from a build stage)
# Here we copy a local binary, but in production you'd likely curl the release
COPY ./target/release/ntnt /usr/local/bin/ntnt
RUN chmod +x /usr/local/bin/ntnt

WORKDIR /app

# Copy application files
COPY . .

# Expose the port your app listens on
EXPOSE 3000

# Run the app
CMD ["ntnt", "run", "server.tnt"]
```

### 1. Build and Run Container

Build the image and run it. We map port 3000 to the host, and optionally mount the `public` folder if we want Nginx to host static files directly.

```bash
docker build -t ntnt-app .
docker run -d \
  --name ntnt-app \
  -p 3000:3000 \
  -v $(pwd)/public:/var/www/myapp/public \
  --restart always \
  ntnt-app
```

### 2. Configure Nginx (Host)

Create `/etc/nginx/sites-available/myapp.com`. Note that even though the app is in Docker, Nginx proxies to `127.0.0.1:3000` because we exposed the port to the host.

```nginx
server {
    listen 80;
    server_name myapp.com;

    # Serve static files from the mounted volume (optional optimization)
    location /static/ {
        alias /var/www/myapp/public/;
        expires 30d;
    }

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host $host;
        proxy_cache_bypass $http_upgrade;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

Enable the site and reload Nginx:

```bash
sudo ln -s /etc/nginx/sites-available/myapp.com /etc/nginx/sites-enabled/
sudo nginx -t
sudo systemctl reload nginx
```

### 3. Setup SSL with Certbot

Run Certbot to enable HTTPS, auto-configure the certificate, and force redirect HTTP traffic to Port 443.

```bash
sudo apt-get install certbot python3-certbot-nginx
sudo certbot --nginx -d myapp.com
```

Certbot will automatically update your Nginx config to listen on `443 ssl` and add the `301` redirect for port 80.
