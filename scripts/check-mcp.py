#!/usr/bin/env python3
"""Exercise the running browser's MCP bridge against a disposable local fixture."""
import argparse
import json
import os
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--profile', required=True)
parser.add_argument('--binary', default=str(Path(__file__).resolve().parents[1] / 'target/debug/browser'))
parser.add_argument('--shared-fixture', action='store_true')
args = parser.parse_args()
process = subprocess.Popen([args.binary, 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                           text=True, env={**os.environ, 'BROWSER_DATA_DIR': args.profile})
sequence = 0

def rpc(method, params):
    global sequence
    sequence += 1
    process.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': sequence, 'method': method, 'params': params}) + '\n')
    process.stdin.flush()
    response = json.loads(process.stdout.readline())
    assert response['id'] == sequence and 'error' not in response, response
    return response['result']

def tool(name, arguments, error=False):
    result = rpc('tools/call', {'name': name, 'arguments': arguments})
    assert result['isError'] == error, result
    return result['content'][0]['text'] if error else json.loads(result['content'][0]['text'])

try:
    rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                       'clientInfo': {'name': 'browser-fixture-check', 'version': '1'}})
    listed = rpc('tools/list', {})
    assert len(listed['tools']) == 5
    tabs = tool('browser_tabs', {})['tabs']
    if not args.shared_fixture:
        assert tabs == [], 'Revoke test tab sharing before the default check.'
        tool('browser_read_page', {'tab_id': 1}, error=True)
        print('PASS: tools advertised, no tabs exposed, unshared read denied')
    else:
        assert len(tabs) == 1 and tabs[0]['url'] == 'http://127.0.0.1:8765/', 'Share only the localhost fixture.'
        tab = {'tab_id': tabs[0]['tab_id']}
        page = tool('browser_read_page', tab)
        assert page['title'] == 'Browser interaction test'
        assert 'Native shell. Real web.' in page['text']
        assert tool('browser_click', {**tab, 'selector': '#increment'})['clicked']
        assert tool('browser_fill', {**tab, 'selector': '#draft', 'value': 'MCP fixture draft'})['filled']
        tool('browser_click', {**tab, 'selector': 'a'}, error=True)
        tool('browser_navigate', {**tab, 'url': 'http://localhost:8765/'}, error=True)
        tool('browser_read_page', {**tab, 'unexpected': True}, error=True)
        print('PASS: shared read/click/fill, ambiguous selector rejection, cross-origin rejection, argument validation')
finally:
    process.stdin.close()
    process.wait(timeout=12)
