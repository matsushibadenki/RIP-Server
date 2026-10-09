#!/usr/bin/env python3
"""Register a disposable local CUPS PDF queue, submit a fixture and verify TIFF completion.
Requires built Rust binaries, running Docker and jrip-mupdf:local. No physical output.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid


def run(args, env, timeout=20):
    result = subprocess.run(args, env=env, stdin=subprocess.DEVNULL,
                            capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f'{args[0]} failed: {result.stderr.strip()}')
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ui-hold-seconds', type=int, default=0,
                        help='Keep successful test queue briefly for settings inspection (0..120).')
    parser.add_argument('--manual-submit', action='store_true',
                        help='Wait for one application print instead of invoking lp. Select the printed temporary queue explicitly.')
    parser.add_argument('--submit-timeout', type=int, default=180,
                        help='Manual submission timeout in seconds (1..900).')
    parser.add_argument('--report', type=Path, help='Save verification evidence outside temporary data.')
    args = parser.parse_args()
    if not 1 <= args.submit_timeout <= 900:
        parser.error('submit timeout must be between 1 and 900 seconds')
    if not 0 <= args.ui_hold_seconds <= 120:
        parser.error('ui hold must be between 0 and 120 seconds')
    repo = Path(__file__).resolve().parent.parent
    binaries = repo / 'target/debug'
    corpus = repo / 'tests/corpus/colors.pdf'
    queue = 'JRIP_Proof_Test_' + uuid.uuid4().hex[:10]
    with tempfile.TemporaryDirectory(prefix='jrip-native-cups-') as root:
        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1', 0))
            port = reservation.getsockname()[1]
        uri = f'ipp://127.0.0.1:{port}/ipp/print'
        env = dict(os.environ, JRIP_DATA_ROOT=root,
                   JRIP_IPP_LISTEN=f'127.0.0.1:{port}', JRIP_PRINTER_URI=uri,
                   CUPS_SERVER=('/private/var/run/cupsd' if sys.platform == 'darwin' else 'localhost'), TMPDIR=root)
        env.pop('JRIP_MDNS', None)
        log = open(Path(root) / 'ipp.log', 'w+')
        server = subprocess.Popen([str(binaries / 'jrip-ipp')], env=env,
                                  stdout=log, stderr=log)
        registered = False
        try:
            for _ in range(50):
                if server.poll() is not None:
                    raise RuntimeError('IPP server exited')
                try:
                    with urllib.request.urlopen(f'http://127.0.0.1:{port}/', timeout=1):
                        break
                except OSError:
                    time.sleep(.1)
            else:
                raise RuntimeError('IPP server startup timed out')
            run(['lpadmin', '-p', queue, '-E', '-v', uri, '-P',
                 str(repo / 'adapters/cups/jrip-pdf-proof.ppd'), '-D',
                 'J-RIP PDF Proof (temporary test)', '-o', 'printer-is-shared=false'], env)
            registered = True
            print(f'Temporary CUPS queue: {queue}', flush=True)
            if args.manual_submit:
                print(f'Open {corpus} in your application and print ALL TWO pages, one copy, to {queue}.', flush=True)
                print('Check the queue name before confirming. Do not select a physical printer.', flush=True)
            else:
                request = run(['lp', '-d', queue, str(corpus)], env)
                print(request.strip(), flush=True)
            deadline = time.monotonic() + (args.submit_timeout if args.manual_submit else 30)
            job = None
            while time.monotonic() < deadline:
                with sqlite3.connect(Path(root) / 'jobs.sqlite3') as connection:
                    rows = connection.execute('SELECT payload FROM jobs').fetchall()
                if rows:
                    if len(rows) != 1:
                        raise RuntimeError('Native client submitted duplicate RIP jobs')
                    job = json.loads(rows[0][0])
                    break
                time.sleep(.2)
            if job is None:
                raise RuntimeError('Native CUPS did not deliver PDF')
            received = (Path(root) / 'jobs' / job['id'] / 'document').read_bytes()
            received_hash = hashlib.sha256(received).hexdigest()
            if not received.startswith(b'%PDF-') or received_hash != job['source_sha256']:
                raise RuntimeError('Received PDF signature or stored source hash mismatch')
            fixture_hash = hashlib.sha256(corpus.read_bytes()).hexdigest()
            if not args.manual_submit and received_hash != fixture_hash:
                raise RuntimeError('PDF passthrough changed source bytes')
            run([str(binaries / 'jrip-dispatcher'), '--once'], env, timeout=90)
            with sqlite3.connect(Path(root) / 'jobs.sqlite3') as connection:
                final = json.loads(connection.execute('SELECT payload FROM jobs').fetchone()[0])
            if final['state'] != 'COMPLETED':
                raise RuntimeError(f'RIP failed: {final["state"]}')
            pages = sorted((Path(root) / 'jobs' / job['id'] / 'raster').glob('page-*.tiff'))
            if len(pages) != 2 or any(p.stat().st_size == 0 for p in pages):
                raise RuntimeError('Expected two nonempty TIFF pages')
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                pending = run(['lpstat', '-W', 'not-completed', '-o', queue], env)
                if not pending.strip():
                    break
                time.sleep(.2)
            else:
                raise RuntimeError('CUPS did not observe remote completion')
            with sqlite3.connect(Path(root) / 'jobs.sqlite3') as connection:
                if connection.execute('SELECT COUNT(*) FROM jobs').fetchone()[0] != 1:
                    raise RuntimeError('Unexpected duplicate submission during completion')
            report = {'schema_version': 1,
                      'submission_mode': 'manual-application' if args.manual_submit else 'lp-passthrough',
                      'job_id': job['id'], 'state': final['state'],
                      'source_sha256': received_hash, 'fixture_sha256': fixture_hash,
                      'source_unchanged': received_hash == fixture_hash,
                      'tiff_pages': len(pages), 'cups_completed': True}
            if args.report:
                args.report.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
            print('PASS: native CUPS -> verified PDF -> MuPDF -> 2 TIFF pages -> CUPS completion', flush=True)
            if args.ui_hold_seconds:
                time.sleep(args.ui_hold_seconds)
        finally:
            try:
                if registered:
                    run(['lpadmin', '-x', queue], env)
                    print('Temporary CUPS queue removed', flush=True)
            finally:
                server.send_signal(signal.SIGINT)
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()
                log.close()


if __name__ == '__main__':
    main()
