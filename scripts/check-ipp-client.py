#!/usr/bin/env python3
"""Portable PDF-only IPP acceptance check (Python 3, no printer installation)."""
import argparse
import hashlib
import http.client
import json
from pathlib import Path
import struct
import time
from urllib.parse import urlsplit


def attribute(tag, name, value):
    name = name.encode('utf-8')
    if isinstance(value, str):
        value = value.encode('utf-8')
    return bytes([tag]) + struct.pack('!H', len(name)) + name + struct.pack('!H', len(value)) + value


def decode(data, request_id):
    if len(data) < 9 or len(data) > 65536:
        raise ValueError('Invalid IPP response size')
    major, minor, status, received = struct.unpack('!BBHI', data[:8])
    if (major, minor) not in ((1, 1), (2, 0)) or received != request_id:
        raise ValueError('IPP version/request ID mismatch')
    offset, name, group, values = 8, None, None, {}
    while offset < len(data):
        tag = data[offset]
        offset += 1
        if tag == 3:
            if offset != len(data):
                raise ValueError('Unexpected response document')
            return status, values
        if tag < 16:
            group, name = tag, None
            continue
        if group is None or offset + 2 > len(data):
            raise ValueError('Truncated IPP attribute')
        size = struct.unpack_from('!H', data, offset)[0]
        offset += 2
        if offset + size + 2 > len(data):
            raise ValueError('Truncated IPP name')
        if size:
            name = data[offset:offset + size].decode('utf-8')
        if name is None:
            raise ValueError('Unnamed IPP value')
        offset += size
        size = struct.unpack_from('!H', data, offset)[0]
        offset += 2
        if offset + size > len(data):
            raise ValueError('Truncated IPP value')
        value = data[offset:offset + size]
        offset += size
        if tag in (0x21, 0x23):
            if size != 4:
                raise ValueError('Invalid IPP integer')
            value = struct.unpack('!i', value)[0]
        elif tag == 0x22:
            if value not in (b'\0', b'\1'):
                raise ValueError('Invalid IPP boolean')
            value = bool(value[0])
        elif 0x41 <= tag <= 0x49:
            value = value.decode('utf-8')
        else:
            value = value.hex()
        values.setdefault(name, []).append(value)
    raise ValueError('Missing IPP end tag')


class Client:
    def __init__(self, uri):
        self.uri = uri
        self.url = urlsplit(uri)
        if (self.url.scheme != 'ipp' or not self.url.hostname or
                self.url.path != '/ipp/print' or self.url.username or
                self.url.password or self.url.query or self.url.fragment):
            raise ValueError('Expected ipp://HOST:PORT/ipp/print without credentials')
        self.request_id = 0

    def call(self, operation, extra=b'', document=b''):
        self.request_id += 1
        body = struct.pack('!BBHI', 2, 0, operation, self.request_id) + b'\1'
        body += attribute(0x47, 'attributes-charset', 'utf-8')
        body += attribute(0x48, 'attributes-natural-language', 'en')
        body += attribute(0x45, 'printer-uri', self.uri)
        body += extra + b'\3' + document
        connection = http.client.HTTPConnection(self.url.hostname, self.url.port or 631, timeout=35)
        try:
            connection.request('POST', self.url.path, body, {'Content-Type': 'application/ipp'})
            response = connection.getresponse()
            data = response.read(65537)
            if response.status != 200 or response.getheader('Content-Type', '').split(';')[0] != 'application/ipp':
                raise RuntimeError(f'Unexpected HTTP response: {response.status}')
            status, values = decode(data, self.request_id)
            if status >= 0x100:
                raise RuntimeError(f'IPP operation 0x{operation:04x} failed: 0x{status:04x}')
            return values
        finally:
            connection.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('uri')
    parser.add_argument('--pdf', type=Path, help='Explicitly submit this PDF; default only queries capabilities.')
    parser.add_argument('--cancel', action='store_true', help='Cancel only the job created by this invocation.')
    parser.add_argument('--wait-seconds', type=int, default=120)
    parser.add_argument('--report', type=Path, help='Write JSON acceptance evidence.')
    args = parser.parse_args()
    if args.cancel and args.pdf is None:
        parser.error('--cancel requires --pdf')
    if not 1 <= args.wait_seconds <= 3600:
        parser.error('--wait-seconds must be 1..3600')
    client = Client(args.uri)
    capabilities = client.call(0x0b)
    if 'application/pdf' not in capabilities.get('document-format-supported', []):
        raise RuntimeError('Printer does not advertise PDF')
    report = {'schema_version': 1, 'client': 'python-direct-ipp',
              'standard_driver_tested': False, 'capabilities': capabilities}
    if args.pdf:
        with args.pdf.open('rb') as source:
            pdf = source.read(32 * 1024 * 1024 + 1)
        if not pdf.startswith(b'%PDF-') or len(pdf) > 32 * 1024 * 1024:
            raise ValueError('Expected a PDF no larger than 32 MiB')
        settings = attribute(0x49, 'document-format', 'application/pdf')
        client.call(0x04, settings)
        job = client.call(0x02, settings, pdf)
        job_id = job['job-id'][0]
        report.update(job_id=job_id, source_sha256=hashlib.sha256(pdf).hexdigest())
        selector = attribute(0x21, 'job-id', struct.pack('!i', job_id))
        if args.cancel:
            client.call(0x08, selector)
        deadline = time.monotonic() + args.wait_seconds
        while True:
            result = client.call(0x09, selector)
            state = result['job-state'][0]
            if state in (7, 8, 9):
                report['job'] = result
                if state != (7 if args.cancel else 9):
                    raise RuntimeError(f'Unexpected terminal job state: {state}')
                break
            if time.monotonic() >= deadline:
                raise TimeoutError(f'Job {job_id} has not reached terminal state; no automatic resubmission')
            time.sleep(1)
    rendered = json.dumps(report, ensure_ascii=False, indent=2)
    if args.report:
        args.report.write_text(rendered + '\n', encoding='utf-8')
    print(rendered)


if __name__ == '__main__':
    main()
