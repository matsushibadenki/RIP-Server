FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ghostscript python3 fonts-noto-cjk && rm -rf /var/lib/apt/lists/*
COPY adapters/ghostscript/render.py /usr/local/lib/jrip/render.py
COPY adapters/stream_pages.py /usr/local/lib/jrip/stream_pages.py
USER 1000:1000
ENTRYPOINT ["python3", "-I", "/usr/local/lib/jrip/render.py"]
