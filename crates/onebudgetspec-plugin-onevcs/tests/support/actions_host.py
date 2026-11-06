#!/usr/bin/env python3
"""GitHub as it answers the Actions token, and the `gh` executable that forwards to it.

`serve STATE` answers from the STATE json file and prints its port once listening. Any other
arguments are one `gh api` call, sent to ACTIONS_HOST. The two spellings below are the point:
GraphQL names the viewer `github-actions[bot]` and the comments it wrote `github-actions`.
"""
import http.server
import json
import os
from pathlib import Path
import re
import sys
import urllib.error
import urllib.request

# llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] AGENTS.md excludes live tests, so no gate can reach GitHub; onevcs owns this wire contract and tests these spellings at its own boundary.
VIEWER = 'github-actions[bot]'
AUTHOR = {'__typename': 'Bot', 'login': 'github-actions'}
# llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]


def serve(path):
    class Host(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            payload = self.payload()
            state = json.loads(path.read_text())
            if self.path == '/graphql':
                if payload.get('variables') != {'owner': 'owner', 'name': 'repo', 'number': 1}:
                    return self.refuse(422, 'unexpected variables: %s' % payload.get('variables'))
                return self.reply(self.read_comments(state))
            if self.path != '/repos/owner/repo/issues/1/comments':
                return self.refuse(404, 'no route: POST %s' % self.path)
            number = 100 + len(state['comments'])
            comment = {'id': 'node-%d' % number, 'databaseId': number, 'author': AUTHOR,
                       'url': 'https://github.com/owner/repo/pull/1#issuecomment-%d' % number}
            state['comments'].append(comment)
            self.write_comment(state, comment, payload)

        def do_PATCH(self):
            payload = self.payload()
            state = json.loads(path.read_text())
            route = re.fullmatch(r'/repos/owner/repo/issues/comments/(\d+)', self.path)
            if not route:
                return self.refuse(404, 'no route: PATCH %s' % self.path)
            comment = next((c for c in state['comments'] if c['databaseId'] == int(route[1])), None)
            if comment is None:
                return self.refuse(404, 'no comment %s' % route[1])
            self.write_comment(state, comment, payload)

        def payload(self):
            return json.loads(self.rfile.read(int(self.headers['Content-Length'])))

        # llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] AGENTS.md excludes live tests, so no gate can reach GitHub; these are only the fields onevcs reads, and onevcs owns that contract.
        def read_comments(self, state):
            comments = {'nodes': state['comments'],
                        'pageInfo': {'hasNextPage': False, 'endCursor': None}}
            return {'data': {'viewer': {'login': VIEWER}, 'repository': {'pullRequest': {
                'headRefOid': state['head'], 'comments': comments}}}}

        def write_comment(self, state, comment, payload):
            if not isinstance(payload.get('body'), str):
                return self.refuse(422, 'body must be a string')
            comment['body'] = payload['body']
            path.write_text(json.dumps(state))
            self.reply({'node_id': comment['id'], 'html_url': comment['url'],
                        'body': comment['body'], 'user': {'login': VIEWER}})
        # llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

        def refuse(self, status, message):
            self.reply({'message': message}, status)

        def reply(self, answer, status=200):
            body = json.dumps(answer).encode()
            self.send_response(status)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Host)
    print(server.server_port, flush=True)
    server.serve_forever()


def gh(args):
    assert args.pop(0) == 'api', args
    method = 'POST'
    fields = {}
    path = None
    while args:
        arg = args.pop(0)
        if arg == '--method':
            method = args.pop(0)
        elif arg in ('-f', '-F'):
            key, value = args.pop(0).split('=', 1)
            fields[key] = int(value) if arg == '-F' else value
        elif path is None and not arg.startswith('-'):
            path = arg
        else:
            sys.exit('unsupported gh argument: %s' % arg)
    if path is None:
        sys.exit('gh api needs an endpoint')
    if path == 'graphql':
        fields = {'query': fields.pop('query'), 'variables': fields}
    request = urllib.request.Request(os.environ['ACTIONS_HOST'] + '/' + path,
                                     data=json.dumps(fields).encode(), method=method)
    try:
        sys.stdout.buffer.write(urllib.request.urlopen(request).read())
    except urllib.error.HTTPError as error:
        sys.exit('HTTP %d: %s' % (error.code, error.read().decode()))


if sys.argv[1:2] == ['serve']:
    serve(Path(sys.argv[2]))
else:
    gh(sys.argv[1:])
