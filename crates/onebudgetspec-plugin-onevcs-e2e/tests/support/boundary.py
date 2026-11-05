#!/usr/bin/env python3
"""The external GitHub HTTP boundary, and the `gh` executable that forwards to it.

`serve ROOT` answers from ROOT/state.json and logs requests to ROOT/requests.jsonl. Any other
arguments are one `gh api` call, sent to BOUNDARY_HOST exactly as `gh` would send it.
The run listing deliberately answers unfiltered: real onevcs decides what is eligible.
"""
import http.server
import json
import os
from pathlib import Path
import re
import sys
import urllib.error
import urllib.request


def serve(root):
    class Host(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        do_GET = do_POST = do_PATCH = lambda self: self.answer()

        def reply(self, status, body):
            self.send_response(status)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def answer(self):
            body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
            payload = json.loads(body) if body else {}
            state = json.loads((root / 'state.json').read_text())
            objects = re.fullmatch(r'/objects/(\d+)', self.path)
            if objects:
                return self.reply(200, (root / 'archives' / (objects[1] + '.zip')).read_bytes())
            with (root / 'requests.jsonl').open('a') as log:
                log.write(json.dumps({'method': self.command, 'path': self.path}) + '\n')
            if self.headers.get('Authorization', '') in ('', 'Bearer '):
                return self.fail(401, 'Bad credentials')
            if self.path == '/graphql':
                return self.graphql(state, payload)
            if self.command in ('POST', 'PATCH'):
                return self.write_comment(root, state, payload)
            return self.read_actions(state)

        def fail(self, status, message):
            self.reply(status, json.dumps({'message': message}).encode())

        def graphql(self, state, payload):
            if 'read_status' in state:
                return self.fail(state['read_status'], 'read refused')
            variables = payload['variables']
            nodes = [dict(id=c['node_id'], databaseId=c['id'], url=c['html_url'], body=c['body'],
                          author={'login': c['author']}) for c in state['comments']]
            self.reply(200, json.dumps({'data': {'viewer': {'login': 'viewer'}, 'repository': {
                'pullRequest': {'headRefOid': state['head'], 'comments': {
                    'nodes': nodes, 'pageInfo': {'hasNextPage': False, 'endCursor': None}}}}}}
                ).encode() if variables['number'] == 1 else b'{}')

        def write_comment(self, root, state, payload):
            if 'write_status' in state:
                return self.fail(state['write_status'], 'Resource not accessible by integration')
            if self.command == 'POST':
                number = 100 + len(state['comments'])
                comment = dict(id=number, node_id='owned-%d' % number, author='viewer',
                               html_url='https://github.com/owner/repo/pull/1#issuecomment-%d' % number)
                state['comments'].append(comment)
            else:
                comment = next(c for c in state['comments'] if str(c['id']) == self.path.split('/')[-1])
            comment['body'] = payload['body']
            (root / 'state.json').write_text(json.dumps(state))
            self.reply(200, json.dumps(comment).encode())

        def read_actions(self, state):
            path = self.path.split('?')[0]
            if '/workflows/' in path:
                return self.reply(200, json.dumps({'workflow_runs': state['runs']}).encode())
            listed = re.fullmatch(r'/repos/owner/repo/actions/runs/(\d+)/artifacts', path)
            if listed and 'list_status' in state:
                return self.fail(state['list_status'], 'artifact listing failed')
            if listed:
                artifacts = state['artifacts'].get(listed[1], [])
                return self.reply(200, json.dumps({'artifacts': artifacts}).encode())
            archive = re.fullmatch(r'/repos/owner/repo/actions/artifacts/(\d+)/zip', path)
            if archive:
                self.send_response(302)
                self.send_header('Location', '/objects/' + archive[1])
                self.send_header('Content-Length', '0')
                return self.end_headers()
            self.fail(404, 'Not Found')

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Host)
    (root / 'port.ready').write_text(str(server.server_port) + '\n')
    (root / 'port.ready').replace(root / 'port')
    server.serve_forever()


def gh(args):
    assert args.pop(0) == 'api', args
    include = '--include' in args
    method = 'GET'
    fields = {}
    path = None
    while args:
        arg = args.pop(0)
        if arg == '--method':
            method = args.pop(0)
        elif arg in ('-f', '-F'):
            key, value = args.pop(0).split('=', 1)
            fields[key] = int(value) if arg == '-F' else value
        elif arg != '--include':
            path = arg
    if path == 'graphql':
        method = 'POST'
        fields = {'query': fields.pop('query'), 'variables': fields}
    request = urllib.request.Request(
        os.environ['BOUNDARY_HOST'] + '/' + path,
        data=json.dumps(fields).encode() if fields else None,
        headers={'Authorization': 'Bearer ' + os.environ.get('GH_TOKEN', '')},
        method=method)
    try:
        response = urllib.request.urlopen(request)
    except urllib.error.HTTPError as error:
        response = error
    data = response.read()
    if include:
        head = ['HTTP/1.1 %d Host' % response.status] + ['%s: %s' % h for h in response.headers.items()]
        sys.stdout.buffer.write(('\r\n'.join(head) + '\r\n\r\n').encode())
        sys.stdout.buffer.write(data)
    elif response.status < 400:
        sys.stdout.buffer.write(data)
    if response.status >= 400:
        sys.stderr.write('HTTP %d: %s\n' % (response.status, data.decode()))
        sys.exit(1)


if sys.argv[1:2] == ['serve']:
    serve(Path(sys.argv[2]))
else:
    gh(sys.argv[1:])
