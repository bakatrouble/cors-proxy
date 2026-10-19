# ---- Builder: compile static-ish release binary on Alpine ----
FROM rust:1-alpine AS builder

RUN apk add --no-cache musl-dev pkgconfig

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
# Cache deps: dummy build to pre-fetch crates
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs && \
    cargo build --release 2>/dev/null || true
COPY src ./src
RUN cargo build --release && \
    strip target/release/cors-proxy || true

# ---- Runner: minimal Alpine image ----
FROM alpine:3 AS runner

RUN apk add --no-cache ca-certificates && \
    addgroup -S app && adduser -S app -G app

COPY --from=builder /app/target/release/cors-proxy /usr/local/bin/cors-proxy

USER app
EXPOSE 8080
ENV PORT=8080
ENV ALLOWED_ORIGINS=*
ENTRYPOINT ["/usr/local/bin/cors-proxy"]
