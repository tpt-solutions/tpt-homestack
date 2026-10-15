# Optional container image for the server. The supported deployment is still the
# single binary on the host with Caddy or Cloudflare in front (see docs/DEPLOYMENT.md).
#
# NOT TESTED in this repository: no Docker was available when it was written. Build
# and run it once before relying on it:
#
#   docker build -t tpt-homestack .
#   docker run --rm -p 8080:8080 -v homestack-data:/data tpt-homestack init --admin-password '...'
#   docker run -d -p 8080:8080 -v homestack-data:/data tpt-homestack
#
# Inside the container the server must listen on all interfaces, so the default
# `host` is overridden here; the port published by `docker run -p` is the only way in.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p tpt-home-cli

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home /data --shell /usr/sbin/nologin homestack \
    && mkdir -p /data \
    && chown homestack /data
COPY --from=build /src/target/release/tpt-home /usr/local/bin/tpt-home
ENV TPT_HOME_DATA_DIR=/data \
    TPT_HOME_HOST=0.0.0.0 \
    TPT_HOME_PORT=8080 \
    TPT_HOME_TRUST_PROXY=false
USER homestack
VOLUME ["/data"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/tpt-home"]
CMD ["run"]
