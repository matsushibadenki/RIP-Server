FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ghostscript python3 python3-venv fonts-noto-cjk && rm -rf /var/lib/apt/lists/*
COPY adapters/source_profiles/requirements.txt /tmp/source-profile-requirements.txt
RUN python3 -m venv /opt/jrip-python && /opt/jrip-python/bin/pip install --no-cache-dir -r /tmp/source-profile-requirements.txt
ENV PATH="/opt/jrip-python/bin:$PATH"
COPY adapters/ghostscript/resources/ /usr/local/lib/jrip/strict-resources/
COPY adapters/ghostscript/render.py /usr/local/lib/jrip/render.py
COPY adapters/font_identity.py /usr/local/lib/jrip/font_identity.py
COPY adapters/font_policy.py /usr/local/lib/jrip/font_policy.py
COPY adapters/page_selection.py /usr/local/lib/jrip/page_selection.py
COPY adapters/stream_pages.py /usr/local/lib/jrip/stream_pages.py
USER 1000:1000
ENTRYPOINT ["python3", "-I", "/usr/local/lib/jrip/render.py"]
