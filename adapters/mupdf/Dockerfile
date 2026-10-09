FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends mupdf-tools python3 fonts-noto-cjk && rm -rf /var/lib/apt/lists/*
COPY render.py /usr/local/lib/jrip/render.py
COPY inspect.py /usr/local/lib/jrip/inspect.py
COPY inspect.js /usr/local/lib/jrip/inspect.js
USER 1000:1000
ENTRYPOINT ["python3", "-I", "/usr/local/lib/jrip/render.py"]
