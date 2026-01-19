# NTNT Language Website - Docker Build
# Multi-stage build for smaller final image

# Stage 1: Build NTNT from source
FROM rust:1.86-bookworm AS builder

WORKDIR /build

# Copy the entire NTNT project
COPY . /ntnt-source

# Build NTNT in release mode
WORKDIR /ntnt-source
RUN cargo build --release

# Stage 2: Runtime image
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create app user for security
RUN useradd -m -u 1000 ntnt

WORKDIR /app

# Copy the built binary
COPY --from=builder /ntnt-source/target/release/ntnt /usr/local/bin/ntnt

# Copy the website files
COPY examples/ntnt-lang-org/server.tnt .
COPY examples/ntnt-lang-org/routes ./routes
COPY examples/ntnt-lang-org/middleware ./middleware
COPY examples/ntnt-lang-org/views ./views
COPY examples/ntnt-lang-org/assets ./assets

# Set ownership
RUN chown -R ntnt:ntnt /app

# Switch to non-root user
USER ntnt

# Expose port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/ || exit 1

# Default timeout (can be overridden)
ENV NTNT_TIMEOUT=30

# Run the server
CMD ["ntnt", "run", "server.tnt"]
