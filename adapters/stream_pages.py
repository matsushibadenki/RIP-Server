"""Linux-only page-close observation; bounded binary frames, never engine logs.

The host still validates every frame. A page observation does not certify that
later pages, interpreter exit, or publication will succeed.
"""
import ctypes
import os
import pathlib
import re
import select
import shutil
import struct
import subprocess
import sys


def stream_pages(command, suffix, max_pages, prepare):
    libc = ctypes.CDLL(None, use_errno=True)
    fd = libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
    if fd < 0:
        raise OSError(ctypes.get_errno(), "inotify_init1")
    child = None
    try:
        # Install before starting the renderer; CLOSE_WRITE is not a size poll.
        if libc.inotify_add_watch(fd, b"/output", 0x8) < 0:
            raise OSError(ctypes.get_errno(), "inotify_add_watch")
        child = subprocess.Popen(command, stdin=subprocess.DEVNULL,
                                 stdout=subprocess.DEVNULL, stderr=sys.stderr)
        output = sys.stdout.buffer
        output.write(b"JRIP-PAGES-1\n")
        output.flush()
        pending, seen, completed = {}, set(), 0
        pattern = re.compile(r"page-([0-9]{6})\." + re.escape(suffix))

        def drain():
            while True:
                try:
                    events = os.read(fd, 65536)
                except BlockingIOError:
                    break
                offset = 0
                while offset < len(events):
                    _, mask, _, length = struct.unpack_from("iIII", events, offset)
                    name = events[offset + 16:offset + 16 + length].split(b"\0", 1)[0]
                    offset += 16 + length
                    if mask & 0x4000:  # IN_Q_OVERFLOW: no reliable count remains.
                        raise ValueError("page event overflow")
                    match = pattern.fullmatch(os.fsdecode(name))
                    if not mask & 0x8 or not match:
                        continue
                    number = int(match[1])
                    if not 1 <= number <= max_pages or number in seen:
                        raise ValueError("page limit or duplicate output")
                    seen.add(number)
                    pending[number] = pathlib.Path("/output") / os.fsdecode(name)

        while True:
            drain()
            while completed + 1 in pending:
                number = completed + 1
                source = pending.pop(number)
                page = prepare(source)
                if page.is_symlink() or not page.is_file():
                    raise ValueError("invalid page")
                size = page.stat().st_size
                if size < 8:
                    raise ValueError("empty page")
                output.write(struct.pack(">IQ", number, size))
                with page.open("rb") as raster:
                    shutil.copyfileobj(raster, output, length=65536)
                output.flush()
                completed = number
            if child.poll() is not None:
                # Events queued immediately before exit must also be drained.
                drain()
                if completed + 1 in pending:
                    continue
                if child.returncode or not completed or pending:
                    raise ValueError("interpreter failed or noncontiguous output")
                output.write(struct.pack(">IQ", 0, 0))
                output.flush()
                return
            select.select([fd], [], [], 0.1)
    finally:
        if child is not None and child.poll() is None:
            child.kill()
            child.wait()
        os.close(fd)
