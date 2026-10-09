"""Bounded single-page color transform; publish a fixed two-file tar stream."""
import os
import subprocess
import sys
import tarfile

output_profile = "/input/output.icc" if os.path.isfile("/input/output.icc") else "-"
with open("/output/report.json", "xb") as report:
    result = subprocess.run(
        ["/usr/local/bin/rip-color", "/input/source.tiff", "/input/input.icc",
         output_profile, "/input/policy.json", sys.argv[1], "/output/result.tiff"],
        stdin=subprocess.DEVNULL, stdout=report, stderr=subprocess.DEVNULL,
        timeout=100, check=False,
    )
if result.returncode:
    sys.exit(1)
for name, limit in [("report.json", 16384), ("result.tiff", 65 * 1024 * 1024)]:
    path = "/output/" + name
    if not os.path.isfile(path) or os.path.islink(path) or not 0 < os.stat(path).st_size <= limit:
        sys.exit(1)
with tarfile.open(fileobj=sys.stdout.buffer, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
    for name in ["report.json", "result.tiff"]:
        archive.add("/output/" + name, arcname=name, recursive=False)
