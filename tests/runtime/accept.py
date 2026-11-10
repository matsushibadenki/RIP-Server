#!/usr/bin/env python3
"""Isolated managed-runtime acceptance. Uses owned sources and a localhost fake printer only."""
import argparse
import hashlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import struct
import sys
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid

WORKSPACE = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('ipp_client', WORKSPACE / 'scripts/check-ipp-client.py')
ipp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ipp)
TOKEN = 'managed-runtime-acceptance-token-at-least-24'
MEDIA = 'custom_square_25.4x25.4mm'


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def pdf():
    stream = '1 0 0 rg 0 0 72 72 re f\n'
    objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
               '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << >> /Contents 4 0 R >>',
               f'<< /Length {len(stream)} >>\nstream\n{stream}endstream']
    data = bytearray(b'%PDF-1.4\n')
    offsets = [0]
    for n, obj in enumerate(objects, 1):
        offsets.append(len(data))
        data.extend(f'{n} 0 obj\n{obj}\nendobj\n'.encode())
    xref = len(data)
    data.extend(f'xref\n0 5\n0000000000 65535 f \n'.encode())
    for offset in offsets[1:]:
        data.extend(f'{offset:010} 00000 n \n'.encode())
    data.extend(f'trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    return bytes(data)


def attributes_end(data):
    offset = 8
    while offset < len(data):
        tag = data[offset]
        offset += 1
        if tag == 3:
            return offset
        if tag < 16:
            continue
        for _ in range(2):
            size = struct.unpack_from('!H', data, offset)[0]
            offset += 2 + size
    raise ValueError('IPP end missing')


class Printer(http.server.BaseHTTPRequestHandler):
    prints = 0
    lock = threading.Lock()
    document = b''

    def log_message(self, *args):
        pass

    def do_POST(self):
        data = self.rfile.read(int(self.headers['content-length']))
        _, _, operation, request_id = struct.unpack('!BBHI', data[:8])
        end = attributes_end(data)
        _, attrs = ipp.decode(data[:end], request_id)
        response = struct.pack('!BBHI', 1, 1, 0, request_id) + b'\1'
        response += ipp.attribute(0x47, 'attributes-charset', 'utf-8')
        response += ipp.attribute(0x48, 'attributes-natural-language', 'en')
        if operation == 0x0b:
            response += b'\4'
            for tag, name, value in [
                (0x49, 'document-format-supported', 'image/pwg-raster'),
                (0x22, 'printer-is-accepting-jobs', b'\1'),
                (0x23, 'printer-state', struct.pack('!i', 3)),
                (0x44, 'printer-state-reasons', 'none'),
                (0x21, 'queued-job-count', struct.pack('!i', 0)),
                (0x33, 'copies-supported', struct.pack('!ii', 1, 5)),
                (0x44, 'sides-supported', 'one-sided'),
                (0x44, 'media-supported', MEDIA),
                (0x32, 'printer-resolution-supported', struct.pack('!iiB', 100, 100, 3)),
                (0x32, 'pwg-raster-document-resolution-supported', struct.pack('!iiB', 100, 100, 3)),
                (0x44, 'pwg-raster-document-type-supported', 'rgb_8')]:
                response += ipp.attribute(tag, name, value)
        elif operation == 2:
            assert attrs['document-format'] == ['image/pwg-raster']
            assert data[end:end+4] == b'RaS2'
            with self.lock:
                type(self).prints += 1
                type(self).document = data[end:]
            response += b'\2' + ipp.attribute(0x21, 'job-id', struct.pack('!i', 42))
            response += ipp.attribute(0x23, 'job-state', struct.pack('!i', 3))
        elif operation == 9:
            response += b'\2' + ipp.attribute(0x21, 'job-id', struct.pack('!i', 42))
            response += ipp.attribute(0x23, 'job-state', struct.pack('!i', 9))
            response += ipp.attribute(0x44, 'job-state-reasons', 'none')
        else:
            raise AssertionError(f'Unexpected device operation {operation}')
        response += b'\3'
        self.send_response(200)
        self.send_header('Content-Type', 'application/ipp')
        self.send_header('Content-Length', str(len(response)))
        self.end_headers()
        self.wfile.write(response)


class Stack:
    def __init__(self, binary, directory):
        self.binary = Path(binary).resolve()
        self.directory = directory
        self.root = directory / 'data'
        self.api_port, self.ipp_port = free_port(), free_port()
        while self.api_port == self.ipp_port:
            self.ipp_port = free_port()
        self.url = f'http://127.0.0.1:{self.api_port}'
        self.ipp_uri = f'ipp://127.0.0.1:{self.ipp_port}/ipp/print'
        self.env = {k: os.environ[k] for k in ['PATH', 'HOME', 'USER', 'DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_CONFIG', 'XDG_RUNTIME_DIR', 'TMPDIR'] if k in os.environ}
        self.env.update(JRIP_API_TOKEN=TOKEN, JRIP_DATA_ROOT=str(self.root), JRIP_BIN_DIR=str(self.binary.parent),
                        JRIP_LISTEN=f'127.0.0.1:{self.api_port}', JRIP_IPP_LISTEN=f'127.0.0.1:{self.ipp_port}',
                        JRIP_PRINTER_URI=self.ipp_uri, JRIP_MDNS='0', RUST_LOG='info')
        self.process = None
        self.serial = 0
        self.files = []

    def request(self, path, value=None, method=None, content_type=None, headers=None):
        if value is not None and not isinstance(value, bytes):
            value = json.dumps(value).encode()
            content_type = 'application/json'
        req = urllib.request.Request(self.url + path, data=value, method=method,
                                     headers={'Authorization': f'Bearer {TOKEN}', **({'Content-Type': content_type} if content_type else {}), **(headers or {})})
        with urllib.request.urlopen(req, timeout=30) as response:
            data = response.read(40 * 1024 * 1024)
            if response.headers.get_content_type() == 'application/json':
                return json.loads(data)
            return data

    def start(self, physical=False):
        self.serial += 1
        self.out_path = self.directory / f'run-{self.serial}.stdout'
        self.err_path = self.directory / f'run-{self.serial}.stderr'
        out = self.out_path.open('wb'); err = self.err_path.open('wb')
        self.files.extend([out, err])
        self.process = subprocess.Popen([str(self.binary)] + (['--with-printers'] if physical else []),
                                        env=self.env, stdout=out, stderr=err, start_new_session=True)
        def ready():
            if self.process.poll() is not None:
                raise RuntimeError(self.err_path.read_text(errors='replace')[-5000:] + '\n' + '\n'.join(p.name + ':\n' + p.read_text(errors='replace')[-2000:] for p in (self.root / 'logs').glob('*.log')))
            for line in self.out_path.read_text().splitlines():
                if json.loads(line).get('status') == 'ready':
                    return True
            return False
        wait(ready, 'runtime ready', 65)
        assert self.request('/health')['status'] == 'ok'
        assert b'J-RIP' in self.request('/console')

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:
                status = self.process.wait(timeout=90)
            except subprocess.TimeoutExpired:
                # Failure cleanup is confined to child process groups recorded by this isolated stack.
                for line in self.err_path.read_text(errors='replace').splitlines():
                    try:
                        fields = json.loads(line).get('fields', {})
                        pid = fields.get('pid')
                        if isinstance(pid, int) and os.getpgid(pid) == pid:
                            os.killpg(pid, signal.SIGTERM)
                    except (ValueError, ProcessLookupError):
                        pass
                self.process.kill(); self.process.wait()
                raise AssertionError('runtime failed to drain on shutdown')
            assert status == 0, self.err_path.read_text(errors='replace')[-5000:]
        self.process = None

    def submit(self, name, mime, data, **settings):
        manifest = {'name': name, 'format': mime, 'dpi': 100, 'color_mode': 'rgb', **settings}
        boundary = 'jrip-runtime-acceptance'
        body = (f'--{boundary}\r\nContent-Disposition: form-data; name="manifest"\r\n\r\n{json.dumps(manifest)}\r\n'
                f'--{boundary}\r\nContent-Disposition: form-data; name="document"; filename="{name}"\r\nContent-Type: {mime}\r\n\r\n').encode() + data + f'\r\n--{boundary}--\r\n'.encode()
        return self.request('/api/v1/jobs', body, content_type=f'multipart/form-data; boundary={boundary}')

    def job(self, job):
        return self.request('/api/v1/jobs/' + job['id'])


def wait(fn, label, seconds=120):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = fn()
        if result:
            return result
        time.sleep(.15)
    raise AssertionError(f'Timed out: {label}')


def completed(stack, job):
    value = stack.job(job)
    if value['state'] in ('FAILED', 'CANCELLED'):
        raise AssertionError(value)
    return value if value['state'] == 'COMPLETED' else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime', required=True)
    parser.add_argument('--icc', type=Path, required=True)
    args = parser.parse_args()
    printer = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Printer)
    threading.Thread(target=printer.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='jrip-runtime-acceptance-') as temporary:
        stack = Stack(args.runtime, Path(temporary))
        try:
            stack.start()
            document = pdf()
            first = stack.submit('proof.pdf', 'application/pdf', document)
            good = wait(lambda: completed(stack, first), 'automatic PDF RIP')
            artifacts = stack.request(f'/api/v1/jobs/{good["id"]}/artifacts')
            assert len(artifacts) == 1
            artifact = artifacts[0]
            raw = stack.request(f'/api/v1/jobs/{good["id"]}/artifacts/{artifact["name"]}')
            assert hashlib.sha256(raw).hexdigest() == artifact['sha256']
            assert stack.request(f'/api/v1/jobs/{good["id"]}/artifacts/{artifact["name"]}/preview')[:2] == b'BM'
            assert good['selected_engine'] == 'mupdf'
            broken = stack.submit('broken.pdf', 'application/pdf', b'%PDF-1.4\nbroken')
            wait(lambda: stack.job(broken)['state'] == 'FAILED', 'malformed source rejection')
            for name, mime, ps in [('proof.ps','application/postscript',b'%!PS-Adobe-3.0\n<< /PageSize [72 72] >> setpagedevice\n0 1 0 setrgbcolor 0 0 72 72 rectfill showpage\n'),
                                   ('proof.eps','application/eps',b'%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 72 72\n0 0 1 setrgbcolor 0 0 72 72 rectfill showpage\n')]:
                job = stack.submit(name, mime, ps)
                assert wait(lambda: completed(stack, job), name)['selected_engine'] == 'ghostscript'
            # Independent IPP client -> the same automatic dispatcher, no explicit worker command.
            client = ipp.Client(stack.ipp_uri)
            received = client.call(2, ipp.attribute(0x49,'document-format','application/pdf'), document)
            ipp_id = received['job-id'][0]
            def ipp_done():
                value = client.call(9, ipp.attribute(0x21,'job-id',struct.pack('!i',ipp_id)))
                assert value['job-state'][0] not in (7,8), value
                return value['job-state'][0] == 9
            wait(ipp_done, 'IPP PDF to TIFF')
            assert Printer.prints == 0, 'default runtime must not print'
            stack.stop()
            config = stack.directory / 'printers.json'
            config.write_text(json.dumps([{'id':'fake','uri':f'ipp://127.0.0.1:{printer.server_port}/ipp/print','media':[{'name':MEDIA,'widthMicrons':25400,'heightMicrons':25400}],'colorTarget':{'outputProfileSha256':hashlib.sha256(args.icc.read_bytes()).hexdigest(),'deviceLinkSha256':None}}]))
            policy = stack.directory / 'auth.json'
            policy.write_text(json.dumps([{'identity':'operator','token':TOKEN,'scopes':['admin']}]))
            stack.env.update(JRIP_PRINTER_CONFIG=str(config), JRIP_AUTH_POLICY=str(policy))
            stack.start(physical=True)
            assert stack.job(first)['state'] == 'COMPLETED', 'restart lost RIP state'
            profile = stack.request('/api/v1/color/profiles', args.icc.read_bytes(), content_type='application/octet-stream')['sha256']
            workflow = str(uuid.uuid4())
            stack.request(f'/api/v1/jobs/{good["id"]}/pipelines', {'requestId':workflow,'expectedRevision':good['revision'],'printerId':'fake','media':MEDIA,
                          'color':{'inputProfileSha256':profile,'outputProfileSha256':profile,'policy':{'schemaVersion':1,'inputColor':'rgb','outputColor':'rgb','intent':'relative_colorimetric','blackPointCompensation':False,'blackPreservation':'none','tacPercent':None}}})
            def awaiting_approval():
                value = stack.request('/api/v1/pipelines/' + workflow)
                assert value['state'] not in ('FAILED','BLOCKED'), value
                return value if value['state'] == 'WAITING_APPROVAL' else None
            value = wait(awaiting_approval, 'automatic ICC and approval wait')
            assert Printer.prints == 0
            grant = stack.request('/api/v1/approvals', {'identity':'operator','jobId':good['id'],'expectedRevision':good['revision'],'requestId':workflow,'printerId':'fake','media':MEDIA,'colorAttemptId':workflow,'reason':'managed runtime acceptance'})
            stack.request('/api/v1/pipelines/' + workflow + '/resume', {'expectedRevision':value['revision'],'approvalId':grant['id']})
            def delivered():
                value = stack.request('/api/v1/pipelines/' + workflow)
                assert value['state'] not in ('FAILED','BLOCKED','UNKNOWN'), value
                return value['state'] == 'COMPLETED'
            wait(delivered, 'device-confirmed workflow')
            assert Printer.prints == 1 and Printer.document.startswith(b'RaS2')
            cached = stack.request('/api/v1/printers/fake/observations?limit=1')
            assert cached['latest']['snapshot']['acceptingJobs'] is True
            stack.stop(); stack.start(physical=True)
            assert stack.request('/api/v1/pipelines/' + workflow)['state'] == 'COMPLETED'
            time.sleep(2)
            assert Printer.prints == 1, 'restart must not resend completed output'
            stack.stop()
            print('PASS managed runtime: automatic PDF/PS/EPS and IPP RIP, TIFF hash/preview, invalid-source recovery, ICC, approval, one fake-device print, graceful stop/restart and no resend', flush=True)
        except BaseException:
            if hasattr(stack, 'err_path') and stack.err_path.exists():
                print(stack.err_path.read_text(errors='replace')[-5000:], file=sys.stderr)
            if (stack.root / 'logs').exists():
                for log in (stack.root / 'logs').glob('*.log'):
                    print(log.name + ':\n' + log.read_text(errors='replace')[-2000:], file=sys.stderr)
            raise
        finally:
            stack.stop()
            for file in stack.files:
                file.close()
            printer.shutdown(); printer.server_close()


if __name__ == '__main__':
    main()
