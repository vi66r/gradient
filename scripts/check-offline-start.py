#!/usr/bin/env python3
"""Regression: opening an unavailable local page must not abort the browser.

Launches only the supplied build with a temporary profile, then stops that
process. An unused loopback port is checked for connection refusal first.
"""
import argparse
import errno
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', default='dist/Browser.app/Contents/MacOS/browser')
parser.add_argument('--seconds', type=float, default=10)
args = parser.parse_args()
if args.seconds < 3:
    parser.error('--seconds must cover at least three one-second URL polls')

with socket.socket() as unavailable, tempfile.TemporaryDirectory(prefix='browser-offline-check-') as profile:
    unavailable.bind(('127.0.0.1', 0))
    address = unavailable.getsockname()
    unavailable.close()
    with socket.socket() as probe:
        probe.settimeout(1)
        assert probe.connect_ex(address) == errno.ECONNREFUSED, 'Expected a closed local port'
    url = f'http://127.0.0.1:{address[1]}/gpu'
    with tempfile.TemporaryFile(mode='w+') as log:
        process = subprocess.Popen([str(Path(args.binary).resolve()), url],
                                   env={**os.environ, 'BROWSER_DATA_DIR': profile, 'RUST_BACKTRACE': '1'},
                                   stdout=log, stderr=log)
        try:
            try:
                code = process.wait(timeout=args.seconds)
            except subprocess.TimeoutExpired:
                snapshot = json.loads((Path(profile) / 'session.json').read_text())
                assert any(tab['url'] == url for tab in snapshot['tabs']), 'UI timer did not persist the requested URL'
                print('PASS: unavailable page survived initial load and repeated URL polls; UI timer saved the session')
            else:
                log.seek(0)
                raise SystemExit(f'FAIL: browser exited with code {code}\n{log.read()}')
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
