"""Owner-approved ZIP acceptance: isolated HTTP upload, process failure, restart and reimport.

The source is opened read-only. The temporary archive is retained for inspection; it may
contain personal data. Stdout contains counters only. Detailed reports stay in that directory.
Requires a previously built development binary and Python 3, no extra Python packages.
"""
import argparse
import collections
import hashlib
import http.client
import json
import os
from pathlib import Path
import secrets
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('zip', type=Path)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/scrobbl-rs.exe' if os.name == 'nt' else 'target/debug/scrobbl-rs'))
    args = parser.parse_args()
    source = args.zip.resolve(strict=True)
    binary = args.binary.resolve(strict=True)
    metadata = source.stat()
    def digest():
        with source.open('rb') as stream:
            return hashlib.file_digest(stream, 'sha256').hexdigest()
    before_hash = digest()
    directory = Path(tempfile.mkdtemp(prefix='scrobbl-spotify-acceptance-'))
    with socket.socket() as reserved:
        reserved.bind(('127.0.0.1', 0))
        port = reserved.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    env = dict(os.environ, SCROBBL_DATA_DIR=str(directory), SCROBBL_PORT=str(port), SPOTIFY_CLIENT_ID='', SPOTIFY_CLIENT_SECRET='')
    process = None
    log = (directory / 'server.log').open('ab')
    cookie = None
    csrf = None
    def api(path, data=None):
        headers = {'Origin': origin}
        if cookie: headers['Cookie'] = cookie
        if csrf: headers['X-CSRF-Token'] = csrf
        if data is not None: headers['Content-Type'] = 'application/json'
        request = urllib.request.Request(origin + path, data=None if data is None else json.dumps(data).encode(), headers=headers)
        return urllib.request.urlopen(request, timeout=30)
    def start():
        nonlocal process
        process = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
        for _ in range(300):
            if process.poll() is not None: raise RuntimeError('Isolated server failed; inspect the private server log')
            try:
                with api('/api/session'): return
            except OSError: time.sleep(0.1)
        raise RuntimeError('Isolated server startup timed out')
    def stop():
        if process and process.poll() is None:
            process.kill()
            process.wait(timeout=30)
    def upload():
        boundary = secrets.token_hex(20)
        prefix = f'--{boundary}\r\nContent-Disposition: form-data; name="files"; filename="owner-history.zip"\r\nContent-Type: application/zip\r\n\r\n'.encode()
        suffix = f'\r\n--{boundary}--\r\n'.encode()
        connection = http.client.HTTPConnection('127.0.0.1', port, timeout=300)
        connection.putrequest('POST', '/api/import/spotify')
        for key, value in {'Content-Type': f'multipart/form-data; boundary={boundary}', 'Content-Length': str(len(prefix)+metadata.st_size+len(suffix)), 'Cookie': cookie, 'X-CSRF-Token': csrf, 'Origin': origin}.items():
            connection.putheader(key, value)
        connection.endheaders()
        connection.send(prefix)
        with source.open('rb') as stream:
            while chunk := stream.read(65536): connection.send(chunk)
        connection.send(suffix)
        response = connection.getresponse()
        result = json.loads(response.read())
        connection.close()
        if response.status != 200: raise RuntimeError(f'Upload failed ({response.status}); inspect Activities in the isolated archive')
        return result['job_id']
    interruption = None
    def wait(job_id, interrupt=False):
        nonlocal interruption
        deadline = time.monotonic() + 7200
        printed = 0
        while time.monotonic() < deadline:
            with api(f'/api/v1/activities/{job_id}') as response: report = json.load(response)
            job = report['activity']
            if interrupt and interruption is None and job['state'] == 'running' and job['processed'] >= 500:
                interruption = job['processed']
                stop()
                start()
                print(f'Isolated server killed at {interruption} confirmed rows and restarted', flush=True)
            if job['state'] in ('completed', 'failed', 'cancelled'): return report
            if time.monotonic()-printed > 20:
                print(json.dumps({key:job[key] for key in ('phase','state','processed','added','invalid')}), flush=True)
                printed = time.monotonic()
            time.sleep(0.15)
        raise RuntimeError('Import did not finish within the acceptance timeout')
    def totals():
        with sqlite3.connect(f'file:{directory.as_posix()}/archive.db?mode=ro', uri=True) as conn:
            return {table: conn.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0] for table in ('listens','observations','observation_variants','source_occurrences')}
    try:
        start()
        with api('/api/setup', {'password': secrets.token_hex(24)}) as response:
            cookie = response.headers['Set-Cookie'].split(';')[0]
            csrf = json.load(response)['csrf']
        began = time.monotonic()
        first = wait(upload(), interrupt=True)
        first_seconds = time.monotonic()-began
        initial = totals()
        (directory / 'first-report.json').write_text(json.dumps(first,indent=2), encoding='utf-8')
        began = time.monotonic()
        second = wait(upload())
        retry_seconds = time.monotonic()-began
        final = totals()
        (directory / 'reimport-report.json').write_text(json.dumps(second,indent=2), encoding='utf-8')
        assert before_hash == digest() and metadata.st_size == source.stat().st_size and metadata.st_mtime_ns == source.stat().st_mtime_ns, 'Source changed during the trial'
        assert initial['listens'] == final['listens'] and initial['observation_variants'] == final['observation_variants'], 'Reimport changed musical identity counts'
        assert all(second['activity'][key] == 0 for key in ('added','originals','variants')), 'Reimport introduced new musical data'
        summary = {
            'source_bytes': metadata.st_size,
            'source_unchanged': True,
            'interrupted_after_rows': interruption,
            'first_seconds': round(first_seconds,3),
            'reimport_seconds': round(retry_seconds,3),
            'first': first['activity'],
            'reimport': second['activity'],
            'inventory_by_kind': dict(collections.Counter(m['kind'] for m in first['members'])),
            'inventory_by_state': dict(collections.Counter(m['state'] for m in first['members'])),
            'initial_totals': initial,
            'final_totals': final,
        }
        (directory / 'summary.json').write_text(json.dumps(summary,indent=2),encoding='utf-8')
        # No source paths, member names or music metadata in stdout.
        print(json.dumps(summary,indent=2),flush=True)
    finally:
        stop()
        log.close()
        print(f'Private acceptance archive retained at {directory}',flush=True)

if __name__ == '__main__': main()
