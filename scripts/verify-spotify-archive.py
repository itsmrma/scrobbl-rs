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
import shutil
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
    parser.add_argument('--benchmark', action='store_true', help='Uninterrupted import and Overview latency, retaining the isolated archive')
    parser.add_argument('--max-import-seconds', type=float, default=20)
    parser.add_argument('--max-dashboard-ms', type=float, default=100)
    parser.add_argument('--recovery', action='store_true', help='Also create/verify/download/upload/restore the personal archive into a second isolated instance')
    args = parser.parse_args()
    source = args.zip.resolve(strict=True)
    binary = args.binary.resolve(strict=True)
    metadata = source.stat()
    def digest():
        with source.open('rb') as stream:
            hash_value = hashlib.sha256()
            for chunk in iter(lambda: stream.read(65536), b''):
                hash_value.update(chunk)
            return hash_value.hexdigest()
    before_hash = digest()
    directory = Path(tempfile.mkdtemp(prefix='scrobbl-spotify-acceptance-'))
    # Pin the tested executable; rebuilding must not change/lock the running trial.
    runner = directory / binary.name
    shutil.copy2(binary, runner)
    with socket.socket() as reserved:
        reserved.bind(('127.0.0.1', 0))
        port = reserved.getsockname()[1]
    origin = f'http://127.0.0.1:{port}'
    env = dict(os.environ, SCROBBL_DATA_DIR=str(directory), SCROBBL_PORT=str(port), SCROBBL_BIND='127.0.0.1', SCROBBL_EXTERNAL_ORIGIN=origin, SPOTIFY_CLIENT_ID='', SPOTIFY_CLIENT_SECRET='')
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
        return urllib.request.urlopen(request, timeout=300)
    def start():
        nonlocal process
        process = subprocess.Popen([str(runner)], env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
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
    def generate_password():
        return ''.join(secrets.choice('BDFHJLNPRTVXZ') + secrets.choice('bdfhjlnprtvxz') + secrets.choice('2468') + secrets.choice('!@#$%^&*') for _ in range(4))
    def archive_fingerprints(root):
        tables = ('listens','observations','observation_variants','original_payloads',
                  'source_occurrences','source_files','calendars','tracks','artists','albums',
                  'track_artists','entity_links','local_corrections','name_groups','name_group_members',
                  'identity_decisions','conflicts','job_inputs','job_members','artwork','artwork_misses')
        result = {}
        with sqlite3.connect(f'file:{root.as_posix()}/archive.db?mode=ro', uri=True) as conn:
            for table in tables:
                columns = len(conn.execute(f'PRAGMA table_info({table})').fetchall())
                ordering = ','.join(str(index+1) for index in range(columns))
                value = hashlib.sha256()
                for row in conn.execute(f'SELECT * FROM {table} ORDER BY {ordering}'):
                    value.update(json.dumps(row, ensure_ascii=False, separators=(',',':')).encode())
                    value.update(b'\n')
                result[table] = value.hexdigest()
        return result
    def disk_usage(root):
        # The isolated process is stopped before checkpointing; never touches the source ZIP.
        conn = sqlite3.connect(str(root / 'archive.db'))
        conn.execute('PRAGMA wal_checkpoint(TRUNCATE)')
        conn.close()
        return {'database_bytes': (root / 'archive.db').stat().st_size,
                'spool_bytes': sum(f.stat().st_size for f in (root / 'spool').iterdir() if f.is_file())}
    def recovery_trial():
        nonlocal cookie, csrf
        expected = archive_fingerprints(directory)
        with api('/api/v1/preferences') as response: preferences = json.load(response)
        with api('/api/v1/recovery', {'kind':'backup'}) as response: asset_id = json.load(response)['id']
        def ready(identifier):
            for _ in range(3600):
                with api('/api/v1/recovery') as response:
                    asset = next(a for a in json.load(response)['assets'] if a['id'] == identifier)
                if asset['state'] == 'ready': return asset
                if asset['state'] == 'failed': raise AssertionError('Recovery operation failed; inspect private logs')
                time.sleep(0.1)
            raise AssertionError('Backup operation timed out')
        began = time.monotonic()
        saved = ready(asset_id)
        creation_seconds = time.monotonic()-began
        assert saved['verified_at_ms'] is None
        with api(f'/api/v1/recovery/{asset_id}', {'action':'verify'}) as response: verification = json.load(response)['job_id']
        assert wait(verification)['activity']['state'] == 'completed'
        assert ready(asset_id)['verified_at_ms'] is not None
        assert expected == archive_fingerprints(directory), 'Verify backup changed live music'
        package = directory / 'downloaded-backup.zip'
        with api(f'/api/v1/recovery/{asset_id}') as response, package.open('wb') as output:
            shutil.copyfileobj(response, output, 65536)
        stop()
        original_disk = disk_usage(directory)
        destination = directory / 'restored-instance'
        env['SCROBBL_DATA_DIR'] = str(destination)
        cookie, csrf = None, None
        start()
        with api('/api/setup', {'username':'owner','password': generate_password()}) as response:
            cookie = response.headers['Set-Cookie'].split(';')[0]
            csrf = json.load(response)['csrf']
        old_cookie = cookie
        boundary = secrets.token_hex(20)
        prefix = f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="backup.zip"\r\nContent-Type: application/zip\r\n\r\n'.encode()
        suffix = f'\r\n--{boundary}--\r\n'.encode()
        connection = http.client.HTTPConnection('127.0.0.1', port, timeout=300)
        connection.putrequest('POST', '/api/v1/recovery/upload')
        for key,value in {'Origin':origin,'Cookie':cookie,'X-CSRF-Token':csrf,
                          'Content-Type':f'multipart/form-data; boundary={boundary}',
                          'Content-Length':str(len(prefix)+package.stat().st_size+len(suffix))}.items(): connection.putheader(key,value)
        connection.endheaders()
        connection.send(prefix)
        with package.open('rb') as source_package:
            for chunk in iter(lambda:source_package.read(65536),b''): connection.send(chunk)
        connection.send(suffix)
        response = connection.getresponse()
        assert response.status == 200, 'Backup upload failed'
        imported_id = json.loads(response.read())['id']
        connection.close()
        ready(imported_id)
        password = generate_password()
        began = time.monotonic()
        with api('/api/v1/recovery/restore', {'asset_id':imported_id,'password':password,'confirm_replace':True}) as response:
            assert json.load(response)['restart_required']
        stop()
        start()
        try:
            with api('/api/v1/recovery'): pass
            raise AssertionError('Old session survived replacement')
        except urllib.error.HTTPError as error:
            assert error.code == 401
        cookie, csrf = None, None
        with api('/api/login', {'username':'owner','password':password}) as response:
            cookie = response.headers['Set-Cookie'].split(';')[0]
            csrf = json.load(response)['csrf']
        assert cookie != old_cookie
        with api('/api/v1/recovery') as response: assert json.load(response)['paused']
        assert expected == archive_fingerprints(destination), 'Restored normalized data, fingerprints or provenance differ'
        with api('/api/v1/preferences') as response: assert json.load(response) == preferences
        with api('/api/v1/recovery/resume', {}) as response: assert json.load(response)['ok']
        with api('/api/v1/recovery') as response: assert not json.load(response)['paused']
        assert list(destination.glob('recovery-*/previous/archive.db')), 'Previous destination missing'
        stop()
        restored_disk = disk_usage(destination)
        assert before_hash == digest() and metadata.st_size == source.stat().st_size and metadata.st_mtime_ns == source.stat().st_mtime_ns
        return {'backup_bytes':package.stat().st_size,'backup_creation_seconds':round(creation_seconds,3),
                'restore_seconds_including_restart':round(time.monotonic()-began,3),
                'verified_live_unchanged':True,'restored_tables_identical':len(expected),
                'preferences_preserved':True,'old_session_rejected':True,'explicit_resumption_verified':True,
                'source_disk':original_disk,'restored_disk':restored_disk}
    try:
        start()
        with api('/api/setup', {'username':'owner','password': generate_password()}) as response:
            cookie = response.headers['Set-Cookie'].split(';')[0]
            csrf = json.load(response)['csrf']
        began = time.monotonic()
        first = wait(upload(), interrupt=not args.benchmark)
        first_seconds = time.monotonic()-began
        initial = totals()
        (directory / 'first-report.json').write_text(json.dumps(first,indent=2), encoding='utf-8')
        if args.benchmark:
            assert first['activity']['state'] == 'completed', 'Import did not complete'
            dashboard_ms = []
            for _ in range(5):
                began = time.monotonic()
                with api('/api/dashboard') as response: overview = json.load(response)
                dashboard_ms.append(round((time.monotonic()-began)*1000,3))
                assert overview['totals']['records'] == initial['listens']
            assert before_hash == digest() and metadata.st_size == source.stat().st_size and metadata.st_mtime_ns == source.stat().st_mtime_ns, 'Source changed during the trial'
            summary = {'source_bytes':metadata.st_size, 'source_unchanged':True,
                       'first_seconds':round(first_seconds,3), 'dashboard_ms':dashboard_ms,
                       'initial_totals':initial,
                       'processed':first['activity']['processed'], 'invalid':first['activity']['invalid']}
            stop()
            summary['disk'] = disk_usage(directory)
            (directory / 'benchmark.json').write_text(json.dumps(summary,indent=2),encoding='utf-8')
            print(json.dumps(summary,indent=2),flush=True)
            assert first_seconds <= args.max_import_seconds, 'Import exceeds the requested time budget'
            assert max(dashboard_ms) <= args.max_dashboard_ms, 'Overview exceeds the latency budget'
            return
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
        if args.recovery:
            summary['recovery'] = recovery_trial()
        (directory / 'summary.json').write_text(json.dumps(summary,indent=2),encoding='utf-8')
        # No source paths, member names or music metadata in stdout.
        print(json.dumps(summary,indent=2),flush=True)
    finally:
        stop()
        log.close()
        print(f'Private acceptance archive retained at {directory}',flush=True)

if __name__ == '__main__': main()
