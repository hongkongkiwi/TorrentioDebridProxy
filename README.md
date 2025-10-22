# Torrentio Debrid Proxy for Stremio

> **Note**: This is a complete Rust rebuild of the [original Node.js project](https://github.com/IrrelevantSoftware/TorrentioDebridProxy), rewritten for significantly better performance, security, and resource efficiency.

This Stremio addon proxies Real-Debrid streaming links from your configured Torrentio instance through your own server. This means Real Debrid will see the same IP address for all streams from all your clients.

It also caches resolved Real-Debrid stream URLs to improve loading times for subsequent requests and reduce requests on the Torrentio API. Note: Sometimes RD connections can hang open, but it shouldn't affect performance. Let me know if it happens to you.

## Implementation

This project is implemented in **Rust** for optimal performance and security:

- **Ultra-low memory footprint**: 2-5MB runtime (1-3MB with optimized build)
- **Fast startup**: <1ms
- **Minimal Docker images**: 5-20MB
- **Production-ready**: All security features implemented
- **Comprehensive testing**: Unit tests for security-critical functions

> **Note**: A Node.js implementation previously existed but was removed due to critical security vulnerabilities (no path sanitization, timing attack vulnerabilities, API key exposure in logs, no SSRF protection). See FINAL_REVIEW.md for details.

### Docker Build Options

Two Dockerfile variants with different size/convenience tradeoffs:

1. **Dockerfile.optimized** - Ultra-minimal (~5-8MB) - Uses scratch base + UPX compression
2. **Dockerfile.rust** - Standard (~15-20MB) - Uses Alpine + static linking + healthcheck

### Pre-built Docker Images

Pre-built multi-architecture (amd64/arm64) images are available on GitHub Container Registry:

```bash
# Standard build (~15-20MB)
docker pull ghcr.io/hongkongkiwi/torrentiodebridproxy:latest

# Optimized build (~5-8MB)
docker pull ghcr.io/hongkongkiwi/torrentiodebridproxy:latest-optimized

# Specific version
docker pull ghcr.io/hongkongkiwi/torrentiodebridproxy:v1.0.0
docker pull ghcr.io/hongkongkiwi/torrentiodebridproxy:v1.0.0-optimized
```

Images are automatically built and published on every push to `main` (tagged as `:latest`) and on version tags (tagged as `:v*`).

### Performance Metrics

| Metric | Optimized Build | Standard Build |
|--------|-----------------|----------------|
| Memory Usage | 1-3 MB | 2-5 MB |
| Startup Time | <1 ms | <1 ms |
| Binary Size | ~1 MB (UPX) | ~2-3 MB |
| Docker Image | 5-8 MB | 15-20 MB |
| CPU Usage | Minimal | Low |
| Build Time | ~5-10 min | ~3-5 min |

**Memory optimization techniques:**
- MiMalloc allocator (better than system malloc)
- Bounded LRU cache with TTL (max 1000 entries, 1 hour TTL)
- Minimal logging in production
- HTTP/1 only (no HTTP/2 overhead)
- Reduced connection pooling
- Size-optimized compilation flags
- Static linking (no dynamic dependencies)

**Security features:**
- Constant-time API key comparison (timing attack prevention)
- SSRF protection with domain whitelist
- Path traversal protection
- Per-key cache locking (thundering herd prevention)
- Input sanitization and validation
- Comprehensive error handling

See [OPTIMIZATIONS.md](OPTIMIZATIONS.md) for performance details and [SECURITY_FIXES.md](SECURITY_FIXES.md) for security improvements.

## Usage in Stremio

Once the addon server is running and accessible at your `PROXY_SERVER_URL`:

1.  Open Stremio.
2.  Go to the Addons section.
3.  In the search bar (or "Install Addon from URL" field), paste the manifest URL for this proxy addon:
    `{{PROXY_SERVER_URL}}/manifest.json`
    (Replace `{{PROXY_SERVER_URL}}` with the actual URL you configured, e.g., `https://stremio-proxy.yourdomain.com/manifest.json` or `http://your.server.ip:13470/manifest.json`)
4.  Click "Install".

Stremio will now fetch movie/series stream information from this proxy, which in turn fetches from your configured Torrentio and rewrites Real-Debrid links to go through this proxy.

## How It Works

1.  You install this addon in Stremio using `{{PROXY_SERVER_URL}}/manifest.json`.
2.  When you select a movie or series in Stremio, Stremio requests stream information from this addon (e.g., `{{PROXY_SERVER_URL}}/stream/movie/tt123456.json`).
3.  This addon forwards the request to your configured `TORRENTIO_URL` to get the list of available streams.
4.  For each stream that is a Real-Debrid link, this addon rewrites the URL:
    *   Original RD link (example): `https://torrentio.strem.fun/realdebrid/......`
    *   Rewritten link: `{{PROXY_SERVER_URL}}/realdebrid/......`
5.  When Stremio tries to play a rewritten link:
    *   It requests `{{PROXY_SERVER_URL}}/realdebrid/......`.
    *   This addon first resolves the actual Real-Debrid direct file URL from `https://torrentio.strem.fun/realdebrid/......`. This resolved URL is cached.
    *   Then, this addon proxies the video data from the resolved Real-Debrid URL directly to your Stremio client. All streaming traffic from Real-Debrid now passes through your server where this addon is hosted. Real-Debrid will only see a single IP, regardless of the client.
    *   
## Prerequisites

*   A [Real-Debrid](https://real-debrid.com/) account.
*   An existing Torrentio addon configuration (you'll need its URL).
*   [Docker](https://www.docker.com/get-started/) and Docker Compose (recommended for easy setup)
*   (Optional) [Just](https://github.com/casey/just) command runner for simplified operations
*   (Alternative) [Rust](https://rustup.rs/) for local development without Docker

## Quick Start

The fastest way to get started:

```bash
# 1. Copy and configure environment
cp .env.example .env
nano .env  # Edit with your settings

# 2A. Using Just (recommended)
just start

# 2B. Using Docker Compose
docker-compose up -d

# 2C. Using Docker manually
docker build -f Dockerfile.rust -t torrentiodebridproxy:rust .
docker run -d --name torrentio-proxy -p 13470:13470 --env-file .env torrentiodebridproxy:rust
```

📖 **For detailed instructions, see [USAGE.md](USAGE.md)**

## Setup

You need to configure the following environment variables:

### 1. `PROXY_SERVER_URL`
The publicly accessible URL where this addon will be running. Stremio clients must be able to reach this URL.

- **Example:**  
  `https://stremio-proxy.yourdomain.com`

---

### 2. `TORRENTIO_URL`
Your personalized Torrentio addon URL.

- **How to obtain it:**
  1. Go to the Torrentio configuration page (e.g., `https://torrentio.strem.fun`).
  2. Configure it with your Real-Debrid API key and preferred settings.
  3. Right-click the **Install** button and choose **Copy Link Address**. This becomes your `TORRENTIO_URL`.

---

### 3. `API_KEY` (Optional)
If set, all incoming requests must include this key as a query parameter:  
`?api_key=YOUR_API_KEY`

- All outgoing stream URLs generated by the addon will automatically include the `api_key` parameter.
- This helps prevent unauthorized use of your proxy.

> **Important:**  
> If you use `API_KEY`, you must remove and re-add the addon to Stremio using the following format:  
> `{PROXY_SERVER_URL}/manifest.json?api_key={yourkey}`

- **Example:**  
  `API_KEY=mysecurekey`  
  Addon URL:  
  `https://stremio-proxy.yourdomain.com/manifest.json?api_key=mysecurekey`


### Option 1: Docker (Recommended)

I've provided Docker images for both implementations.

**Using Just Commands (Easiest):**
```bash
# Show all available commands
just

# Quick start
just start

# Start optimized version
just start-optimized

# View logs
just docker-logs

# Monitor resources
just docker-stats
```

**Using Docker Compose:**
```bash
# Standard Rust version
docker-compose up -d

# Optimized version
docker-compose --profile optimized up -d

# With VPN (Gluetun)
docker-compose --profile vpn up -d

# View logs
docker-compose logs -f
```

**Manual Docker Build:**
```bash
# Ultra-optimized (smallest: ~5-8MB, no healthcheck)
docker build -f Dockerfile.optimized -t torrentiodebridproxy:optimized .

# Standard (recommended: ~15-20MB, includes healthcheck)
docker build -f Dockerfile.rust -t torrentiodebridproxy:rust .
```

📖 **See [USAGE.md](USAGE.md) for comprehensive deployment options including VPN and Cloudflare Tunnel setups.**

#### A. Bare Bones Docker Compose (No VPN, No Tunnel)

This setup exposes the addon directly on port 13470. You'll need to ensure this port is accessible and `PROXY_SERVER_URL` points to your server's IP/domain and this port (e.g., `http://your.server.ip:13470`).

Note: Any stremio clients needs to be able to access the PROXY_SERVER_URL for this to work. i.e. If its a local IP you're hosting the proxy on, you'll only be able to access it within that network or via VPN/Tailscale etc.

**Using Rust (Recommended):**
`docker-compose.yml`:
```yaml
version: "3.8"

services:
  torrentio-debrid-proxy:
    build:
      context: .
      dockerfile: Dockerfile.rust
    # OR use pre-built image:
    # image: ghcr.io/irrelevantsoftware/torrentiodebridproxy:rust
    container_name: torrentio-debrid-proxy
    restart: unless-stopped
    environment:
      - PROXY_SERVER_URL={{YOUR_PROXY_URL}} # e.g., http://your.server.ip:13470 or https://your.domain.com
      - TORRENTIO_URL={{YOUR_TORRENTIO_URL}} # Your configured Torrentio URL
      - API_KEY={{YOUR_API_KEY}} # Optional but recommended
      # - PORT=13470 # Optional: Change if port 13470 is already in use
      # - RUST_LOG=torrentio_debrid_proxy=info # Optional: Logging level
    ports:
      - "13470:13470" # Exposes the addon on port 13470 (or your custom PORT)
```

Replace `{{YOUR_PROXY_URL}}`, `{{YOUR_TORRENTIO_URL}}`, and `{{YOUR_API_KEY}}` with your actual values.
Then run: `docker-compose up -d`

#### B. Docker Compose with Gluetun VPN and Cloudflare Tunnel

This setup routes the addon's traffic through Gluetun VPN (so Real-Debrid sees the VPN IP, not your server IP) and exposes the addon via a Cloudflare Tunnel, allowing your remote Stremio clients to access it on any network.

NOTE: Depending on your Cloudflare Tunnel setup, this method exposes the addon server publicly - be mindful of your network security.

`docker-compose.yml`:
```yaml
version: "3.8"

services:
  # Stremio Addon (Rust implementation)
  torrentio-debrid-proxy:
    build:
      context: .
      dockerfile: Dockerfile.rust  # or use Dockerfile.optimized for smallest image
    # OR use pre-built image:
    # image: ghcr.io/irrelevantsoftware/torrentiodebridproxy:rust
    container_name: torrentio-debrid-proxy
    restart: unless-stopped
    environment:
      - PROXY_SERVER_URL={{YOUR_TUNNEL_HOSTNAME}} # e.g., https://stremio-proxy.yourdomain.com (from Cloudflare Tunnel)
      - TORRENTIO_URL={{YOUR_TORRENTIO_URL}} # Your configured Torrentio URL
      - API_KEY={{SECURE_KEY}}
      # - PORT=13470 # Optional: Internal port for the addon
      # - RUST_LOG=torrentio_debrid_proxy=info # Optional: For Rust version only
    network_mode: "service:gluetun" # Routes traffic through Gluetun
    depends_on:
      - gluetun

  # Gluetun VPN
  gluetun:
    container_name: "gluetun"
    image: qmcgaw/gluetun
    cap_add:
      - NET_ADMIN
    devices:
      - /dev/net/tun:/dev/net/tun
    ports: # Only needed if you want to access other services through Gluetun directly, not for this addon's primary function
      - 13470:13470/tcp # Expose the addon's port through Gluetun for Cloudflared
    environment:
      - VPN_SERVICE_PROVIDER= # e.g., nordvpn, mullvad, custom
      - VPN_TYPE=wireguard # or openvpn
      # Add your VPN provider specific credentials below
      # For Wireguard:
      - WIREGUARD_PRIVATE_KEY=...
      - WIREGUARD_ADDRESSES=... # e.g., 10.66.169.2/32
      # For OpenVPN:
      # - OPENVPN_USER=...
      # - OPENVPN_PASSWORD=...
      - SERVER_COUNTRIES=... # e.g., Netherlands, Switzerland
    restart: unless-stopped

  # Cloudflare tunnel to expose PROXY_SERVER_URL
  cloudflared:
    container_name: cloudflared
    image: cloudflare/cloudflared
    restart: unless-stopped
    command: tunnel run
    environment:
      - TUNNEL_TOKEN={{CF_TUNNEL_TOKEN}} # Your Cloudflare Tunnel token
    depends_on:
      - gluetun # Ensure Gluetun is up before Cloudflared tries to connect
    network_mode: "service:gluetun" # So Cloudflared can reach the addon via gluetun's network
                                    # Cloudflare Tunnel service should point to http://localhost:13470
                                    # (or http://gluetun:13470 if using default Docker networking and not network_mode:service:gluetun)
                                    # *Correction*: If cloudflared is in network_mode: service:gluetun,
                                    # it will resolve 'localhost' as gluetun itself.
                                    # The service in your Cloudflare Tunnel dashboard should be: http://localhost:13470
                                    # (assuming the addon is listening on port 13470 within the Gluetun network namespace)
```
**Important Notes for Gluetun + Cloudflare setup:**
1.  Replace `{{YOUR_TUNNEL_HOSTNAME}}`, `{{YOUR_TORRENTIO_URL}}`, and `{{CF_TUNNEL_TOKEN}}` with your actual values.
2.  Configure Gluetun with your VPN provider's details.
3.  In your Cloudflare Tunnel configuration (Zero Trust Dashboard), create a public hostname (this will be your `PROXY_SERVER_URL`) and point its service to `http://gluetun:13470` (or `http://localhost:YOUR_CUSTOM_PORT` if you changed the `PORT` environment variable for `torrentio-debrid-proxy`).

Then run: `docker-compose up -d` and add the addon to Stremio via {PROXY_SERVER_URL}/manifest.json?api_key={SECURE_KEY}

NOTE: Depending on your Cloudflare Tunnel setup, this method exposes the addon server publicly - be mindful of your network security.

### Option 2: Build from Source

#### Rust (Recommended)

1.  Clone this repository:
    ```bash
    git clone <repository_url>
    cd <repository_directory>
    ```

2.  Create a `.env` file in the root directory with your configuration:
    ```env
    PROXY_SERVER_URL=https://your-proxy-url.com
    TORRENTIO_URL=https://torrentio.strem.fun/your-config/manifest.json
    API_KEY=your-secret-key
    # PORT=13470 # Optional
    # RUST_LOG=torrentio_debrid_proxy=info # Optional
    ```

3.  Build and run:
    ```bash
    cargo build --release
    cargo run --release
    # Or run the binary directly:
    # ./target/release/torrentio-debrid-proxy
    ```

## License

This project is [MIT](./LICENSE) licensed.
