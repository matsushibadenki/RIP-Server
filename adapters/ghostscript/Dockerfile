FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ghostscript fonts-noto-cjk && rm -rf /var/lib/apt/lists/*
COPY --chmod=755 render.sh /usr/local/bin/jrip-render
USER 1000:1000
ENTRYPOINT ["/usr/local/bin/jrip-render"]
