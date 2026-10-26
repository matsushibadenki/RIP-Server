FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends mupdf-tools python3 python3-venv fonts-noto-cjk && rm -rf /var/lib/apt/lists/*
COPY adapters/source_profiles/requirements.txt /tmp/source-profile-requirements.txt
RUN python3 -m venv /opt/jrip-python && /opt/jrip-python/bin/pip install --no-cache-dir -r /tmp/source-profile-requirements.txt
ENV PATH="/opt/jrip-python/bin:$PATH"
COPY adapters/source_profiles/analyze.py /usr/local/lib/jrip/source_profiles.py
COPY adapters/mupdf/render.py /usr/local/lib/jrip/render.py
COPY adapters/mupdf/inspect.py /usr/local/lib/jrip/inspect.py
COPY adapters/mupdf/inspect.js /usr/local/lib/jrip/inspect.js
COPY adapters/stream_pages.py /usr/local/lib/jrip/stream_pages.py
USER 1000:1000
ENTRYPOINT ["python3", "-I", "/usr/local/lib/jrip/render.py"]
