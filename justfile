# Torrentio Debrid Proxy - Just Commands
# https://github.com/casey/just

# Default recipe to display help
default:
    @just --list

# Variables
rust_image := "torrentiodebridproxy:rust"
optimized_image := "torrentiodebridproxy:optimized"
node_image := "torrentiodebridproxy:node"
container_name := "torrentio-debrid-proxy"

# ============================================================================
# Development Commands
# ============================================================================

# Run Rust version locally (debug mode)
run:
    cargo run

# Run Rust version locally (release mode)
run-release:
    cargo run --release

# Run Node.js version locally
run-node:
    node index.js

# Build Rust binary (debug)
build:
    cargo build

# Build Rust binary (release)
build-release:
    cargo build --release

# Check Rust code without building
check:
    cargo check

# Format Rust code
fmt:
    cargo fmt

# Run Rust linter
lint:
    cargo clippy -- -D warnings

# Run all checks (format, lint, build)
verify: fmt lint build-release
    @echo "✅ All checks passed!"

# Clean build artifacts
clean:
    cargo clean
    rm -rf target/

# ============================================================================
# Docker - Rust Version (Recommended)
# ============================================================================

# Build Rust Docker image (standard)
docker-build-rust:
    docker build -f Dockerfile.rust -t {{rust_image}} .

# Build Rust Docker image (optimized/minimal)
docker-build-optimized:
    docker build -f Dockerfile.optimized -t {{optimized_image}} .

# Run Rust Docker container (requires .env file)
docker-run-rust: docker-build-rust
    docker run -d \
      --name {{container_name}} \
      -p 13470:13470 \
      --env-file .env \
      --restart unless-stopped \
      {{rust_image}}

# Run optimized Rust Docker container (requires .env file)
docker-run-optimized: docker-build-optimized
    docker run -d \
      --name {{container_name}} \
      -p 13470:13470 \
      --env-file .env \
      --restart unless-stopped \
      {{optimized_image}}

# Run Rust Docker container with memory limit (10MB)
docker-run-rust-limited: docker-build-rust
    docker run -d \
      --name {{container_name}} \
      -p 13470:13470 \
      --env-file .env \
      --memory="10m" \
      --memory-swap="10m" \
      --restart unless-stopped \
      {{rust_image}}

# Run Rust Docker container interactively (for testing)
docker-run-rust-interactive: docker-build-rust
    docker run -it --rm \
      --name {{container_name}}-test \
      -p 13470:13470 \
      --env-file .env \
      {{rust_image}}

# ============================================================================
# Docker - Node.js Version (Legacy)
# ============================================================================

# Build Node.js Docker image
docker-build-node:
    docker build -f Dockerfile -t {{node_image}} .

# Run Node.js Docker container (requires .env file)
docker-run-node: docker-build-node
    docker run -d \
      --name {{container_name}} \
      -p 13470:13470 \
      --env-file .env \
      --restart unless-stopped \
      {{node_image}}

# ============================================================================
# Docker - Management
# ============================================================================

# Stop the running container
docker-stop:
    docker stop {{container_name}} || true

# Remove the container
docker-rm: docker-stop
    docker rm {{container_name}} || true

# Restart the container
docker-restart:
    docker restart {{container_name}}

# View container logs
docker-logs:
    docker logs -f {{container_name}}

# View container logs (last 100 lines)
docker-logs-tail:
    docker logs --tail 100 {{container_name}}

# Show container stats (memory, CPU usage)
docker-stats:
    docker stats {{container_name}}

# Execute shell in running container (Rust - Alpine)
docker-shell:
    docker exec -it {{container_name}} /bin/sh

# Execute shell in running container (Node.js)
docker-shell-node:
    docker exec -it {{container_name}} /bin/bash

# Inspect container details
docker-inspect:
    docker inspect {{container_name}}

# Clean up all related Docker resources
docker-clean: docker-rm
    docker rmi {{rust_image}} {{optimized_image}} {{node_image}} 2>/dev/null || true
    docker system prune -f

# ============================================================================
# Docker Compose
# ============================================================================

# Start services with docker-compose
compose-up:
    docker-compose up -d

# Start services and show logs
compose-up-logs:
    docker-compose up

# Stop services
compose-down:
    docker-compose down

# Restart services
compose-restart:
    docker-compose restart

# View logs
compose-logs:
    docker-compose logs -f

# View logs (tail)
compose-logs-tail:
    docker-compose logs --tail=100 -f

# Show service status
compose-ps:
    docker-compose ps

# Rebuild and restart services
compose-rebuild:
    docker-compose up -d --build

# Clean up compose resources
compose-clean:
    docker-compose down -v --rmi all

# ============================================================================
# Testing & Health Checks
# ============================================================================

# Test manifest endpoint (localhost)
test-manifest:
    curl -s http://localhost:13470/manifest.json | jq .

# Test manifest endpoint with API key
test-manifest-auth API_KEY:
    curl -s "http://localhost:13470/manifest.json?api_key={{API_KEY}}" | jq .

# Test health of running container
test-health:
    @echo "Testing manifest endpoint..."
    @curl -f -s http://localhost:13470/manifest.json > /dev/null && echo "✅ Service is healthy" || echo "❌ Service is not responding"

# Run security checks (path traversal attempt)
test-security:
    @echo "Testing path traversal protection..."
    @curl -s -o /dev/null -w "%{http_code}" "http://localhost:13470/resolve/realdebrid/../../../etc/passwd" | grep -q "400" && echo "✅ Path traversal blocked" || echo "❌ Path traversal not blocked"

# Benchmark memory usage
test-memory:
    @echo "Memory usage of running container:"
    @docker stats {{container_name}} --no-stream --format "table {{{{.Container}}}}\t{{{{.MemUsage}}}}\t{{{{.MemPerc}}}}"

# Full test suite
test-all: test-health test-security test-memory
    @echo "✅ All tests completed"

# ============================================================================
# Environment Setup
# ============================================================================

# Create .env file from example
env-create:
    @if [ ! -f .env ]; then \
        cp .env.example .env; \
        echo "✅ Created .env from .env.example"; \
        echo "⚠️  Please edit .env with your configuration"; \
    else \
        echo "❌ .env already exists"; \
    fi

# Validate .env file exists
env-check:
    @if [ ! -f .env ]; then \
        echo "❌ .env file not found. Run 'just env-create' first"; \
        exit 1; \
    else \
        echo "✅ .env file exists"; \
    fi

# Show current environment configuration (without secrets)
env-show:
    @echo "Current configuration:"
    @cat .env | grep -v "API_KEY" | grep -v "^#" | grep -v "^$"

# ============================================================================
# Quick Start Commands
# ============================================================================

# Quick start - build and run Rust version
start: env-check docker-build-rust docker-rm docker-run-rust
    @echo "✅ Started Rust version"
    @echo "📊 Checking status..."
    @sleep 2
    @just docker-logs-tail

# Quick start - build and run optimized version
start-optimized: env-check docker-build-optimized docker-rm docker-run-optimized
    @echo "✅ Started optimized version"
    @echo "📊 Checking status..."
    @sleep 2
    @just docker-logs-tail

# Quick start with docker-compose
start-compose: env-check compose-up
    @echo "✅ Started with docker-compose"
    @sleep 2
    @just compose-logs-tail

# Stop everything
stop:
    @just docker-stop || true
    @just compose-down || true
    @echo "✅ Stopped all services"

# ============================================================================
# Information & Help
# ============================================================================

# Show project information
info:
    @echo "Torrentio Debrid Proxy"
    @echo "====================="
    @echo ""
    @echo "📦 Docker Images:"
    @docker images | grep -E "torrentiodebridproxy|REPOSITORY" || echo "No images built yet"
    @echo ""
    @echo "🐳 Containers:"
    @docker ps -a | grep -E "{{container_name}}|CONTAINER" || echo "No containers running"
    @echo ""
    @echo "💾 Binaries:"
    @ls -lh target/release/torrentio-debrid-proxy 2>/dev/null || echo "No release binary built yet"

# Show version information
version:
    @echo "Rust version:"
    @rustc --version
    @echo ""
    @echo "Cargo version:"
    @cargo --version
    @echo ""
    @echo "Docker version:"
    @docker --version
    @echo ""
    @echo "Docker Compose version:"
    @docker-compose --version 2>/dev/null || echo "docker-compose not installed"

# Show recommended workflow
help:
    @echo "Torrentio Debrid Proxy - Quick Start Guide"
    @echo "=========================================="
    @echo ""
    @echo "📝 First Time Setup:"
    @echo "  1. just env-create              # Create .env from template"
    @echo "  2. edit .env                    # Add your configuration"
    @echo "  3. just start                   # Build and run"
    @echo ""
    @echo "🚀 Common Commands:"
    @echo "  just start                      # Quick start (Rust version)"
    @echo "  just start-optimized            # Quick start (optimized)"
    @echo "  just start-compose              # Quick start (docker-compose)"
    @echo "  just stop                       # Stop all services"
    @echo "  just docker-logs                # View logs"
    @echo "  just test-all                   # Run tests"
    @echo ""
    @echo "🔧 Development:"
    @echo "  just run                        # Run locally (debug)"
    @echo "  just run-release                # Run locally (release)"
    @echo "  just verify                     # Format, lint, build"
    @echo ""
    @echo "📊 Monitoring:"
    @echo "  just docker-stats               # Monitor resource usage"
    @echo "  just test-memory                # Check memory usage"
    @echo "  just info                       # Show project info"
    @echo ""
    @echo "📖 For full command list: just --list"
