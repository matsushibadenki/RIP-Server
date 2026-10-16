"""Run inside a test container: partial files must not be reported as pages."""
import pathlib
import select
import struct
import subprocess
import sys
import time

renderer = r'''
import pathlib, time
root = pathlib.Path("/output")
with (root / "page-000001.tiff").open("wb") as first:
    first.write(b"II\x2a\x00")
    first.flush()
    (root / "writing").touch()
    while not (root / "finish-first").exists(): time.sleep(0.01)
    first.write(b"\x08\x00\x00\x00")
while not (root / "release-second").exists(): time.sleep(0.01)
(root / "page-000002.tiff").write_bytes(b"II\x2a\x00\x08\x00\x00\x00")
'''
wrapper = "\n".join([
    "import sys",
    "sys.path.insert(0, '/usr/local/lib/jrip')",
    "from stream_pages import stream_pages",
    "stream_pages([sys.executable, '-I', '-c', " + repr(renderer) + "], 'tiff', 2, lambda page: page)",
])
child = subprocess.Popen([sys.executable, "-I", "-c", wrapper], stdout=subprocess.PIPE)
try:
    assert child.stdout.read(13) == b"JRIP-PAGES-1\n"
    deadline = time.monotonic() + 5
    while not pathlib.Path("/output/writing").exists():
        assert time.monotonic() < deadline
        time.sleep(0.01)
    # The producer has flushed a TIFF signature but still holds the page open.
    assert not select.select([child.stdout], [], [], 0.2)[0]
    pathlib.Path("/output/finish-first").touch()
    assert struct.unpack(">IQ", child.stdout.read(12)) == (1, 8)
    assert child.stdout.read(8) == b"II\x2a\x00\x08\x00\x00\x00"
    assert child.poll() is None  # Renderer is blocked before producing page 2.
    assert not pathlib.Path("/output/page-000002.tiff").exists()
    pathlib.Path("/output/release-second").touch()
    assert struct.unpack(">IQ", child.stdout.read(12)) == (2, 8)
    assert child.stdout.read(8) == b"II\x2a\x00\x08\x00\x00\x00"
    assert child.stdout.read(12) == bytes(12)
    assert child.stdout.read() == b""
    assert child.wait(timeout=5) == 0
finally:
    if child.poll() is None:
        child.kill()
        child.wait()
