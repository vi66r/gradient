#!/usr/bin/env python3
"""Exercise agent access end to end through the MCP bridge.

Run a debug build with GRADIENT_TEST_AGENT=share (or trust) and
GRADIENT_DATA_DIR=<profile>, open http://127.0.0.1:8765/agent from
scripts/test-server.py, keep the window on screen, then:

    python3 scripts/check-agent.py --profile <profile> --mode share
"""
import argparse
import base64
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--profile', required=True)
parser.add_argument('--mode', choices=['share', 'trust'], required=True)
parser.add_argument('--binary', default=str(Path(__file__).resolve().parents[1] / 'target/debug/gradient'))
args = parser.parse_args()
process = subprocess.Popen([args.binary, 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                           text=True, env={**os.environ, 'GRADIENT_DATA_DIR': args.profile})
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
    content = result['content'][0]
    assert result['isError'] == error, (name, result)
    if error or content['type'] == 'image':
        return content
    return json.loads(content['text'])


def text(tab):
    return tool('browser_read_page', tab)['text']


def eventually(check, what):
    for _ in range(30):
        if check():
            return
        time.sleep(0.1)
    raise AssertionError(what)


try:
    rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                       'clientInfo': {'name': 'browser-agent-check', 'version': '1'}})
    assert len(rpc('tools/list', {})['tools']) == 12
    tabs = tool('browser_tabs', {})['tabs']
    assert len(tabs) == 1 and tabs[0]['url'] == 'http://127.0.0.1:8765/agent', tabs
    expected = 'trusted' if args.mode == 'trust' else 'shared'
    assert tabs[0]['access'] == expected, tabs
    tab = {'tab_id': tabs[0]['tab_id']}

    page = tool('browser_read_page', tab)
    assert 'Visible paragraph.' in page['text'], page['text']
    hidden = [word for word in page['text'].split() if word.startswith('HIDDEN-')]
    assert not hidden, f'hidden text reached the agent: {hidden}'
    assert 'SHOWN-ARIA' in page['text'], 'aria-hidden text is on screen, so it counts'
    assert any(control['selector'] == '#press' for control in page['controls']), page['controls']
    print('PASS: reads visible text only, past a page that broke querySelectorAll in its own world')

    assert tool('browser_click', {**tab, 'selector': '#press'}) == {'clicked': True, 'input': 'native'}
    eventually(lambda: 'Press trusted' in text(tab), 'click arrived as a real click')
    filled = tool('browser_fill', {**tab, 'selector': '#draft', 'value': 'hello agent'})
    assert filled == {'filled': True, 'input': 'native'}, filled
    eventually(lambda: 'trusted:hello agent' in text(tab), 'typing arrived as real input')
    print('PASS: clicks and typing are real input (isTrusted)')

    tool('browser_fill', {**tab, 'selector': '#file', 'value': 'x'}, error=True)
    tool('browser_click', {**tab, 'selector': 'p'}, error=True)
    tool('browser_read_page', {**tab, 'unexpected': True}, error=True)
    print('PASS: file fields refused, ambiguous selectors refused, arguments validated')

    if args.mode == 'share':
        for name, extra in [('browser_screenshot', {}), ('browser_read_dom', {}), ('browser_console', {}),
                            ('browser_network', {}), ('browser_eval', {'code': 'return 1'}),
                            ('browser_press_key', {'key': 'a'}),
                            ('browser_upload', {'selector': '#file', 'paths': ['/etc/hosts']})]:
            message = tool(name, {**tab, **extra}, error=True)['text']
            assert 'trusted' in message, message
        tool('browser_navigate', {**tab, 'url': 'http://localhost:8765/'}, error=True)
        print('PASS: shared tab: trusted-only tools refused, cross-site navigation refused')
    else:
        shot = tool('browser_screenshot', tab)
        assert shot['type'] == 'image' and base64.b64decode(shot['data'])[:8] == b'\x89PNG\r\n\x1a\n'
        dom = tool('browser_read_dom', tab)
        assert 'HIDDEN-WHITE' in dom['html'] and 'button "Press"' in dom['outline'], dom['outline'][:400]
        assert tool('browser_eval', {**tab, 'code': 'return document.title'}) == 'Agent check'
        messages = tool('browser_console', tab)['messages']
        assert any(message['text'] == 'pressed true' for message in messages), messages
        tool('browser_navigate', {**tab, 'url': 'http://127.0.0.1:8765/agent'})
        time.sleep(1.5)
        messages = tool('browser_console', tab)['messages']
        assert any(message['text'] == 'page loaded' and message['level'] == 'warn' for message in messages), messages
        loads = tool('browser_network', tab)['loads']
        ping = [load for load in loads if load['url'].endswith('/api/agent-ping')]
        assert ping and ping[-1].get('status') == 200, loads
        print('PASS: trusted: screenshot, DOM and outline, eval, console (hooked from document start), network')

        tool('browser_click', {**tab, 'selector': '#draft'})
        assert tool('browser_press_key', {**tab, 'key': 'x'}) == {'pressed': 'x'}
        eventually(lambda: 'trusted:x' in text(tab), 'key press arrived as real input')
        with tempfile.NamedTemporaryFile(suffix='.txt', delete=False) as upload:
            upload.write(b'agent upload')
        name = Path(upload.name).name
        assert tool('browser_upload', {**tab, 'selector': '#file', 'paths': [upload.name]}) == {'uploaded': [name]}
        eventually(lambda: f'{name}:12' in text(tab), 'upload reached the page')
        os.unlink(upload.name)
        assert tool('browser_click', {**tab, 'selector': '#order'})['clicked']
        eventually(lambda: 'ordered' in text(tab), 'trusted click submitted without asking')
        tool('browser_navigate', {**tab, 'url': 'http://localhost:8765/agent'})
        print('PASS: trusted: real key presses, uploads, consequential click without a prompt, any-site navigation')
finally:
    process.stdin.close()
    process.wait(timeout=12)
