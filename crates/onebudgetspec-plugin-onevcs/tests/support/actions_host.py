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
import sys
import urllib.error
import urllib.request

VIEWER = 'github-actions[bot]'
AUTHOR = {'__typename': 'Bot', 'login': 'github-actions'}


def serve(path):
    class Host(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            payload = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            state = json.loads(path.read_text())
            if self.path == '/graphql':
                return self.reply(self.read_comments(state))
            number = 100 + len(state['comments'])
            comment = {'id': 'node-%d' % number, 'databaseId': number, 'author': AUTHOR,
                       'url': 'https://github.com/owner/repo/pull/1#issuecomment-%d' % number}
            state['comments'].append(comment)
            self.write_comment(state, comment, payload)

        def do_PATCH(self):
            payload = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            state = json.loads(path.read_text())
            database_id = int(self.path.rsplit('/', 1)[1])
            comment = next(c for c in state['comments'] if c['databaseId'] == database_id)
            self.write_comment(state, comment, payload)

        def read_comments(self, state):
            comments = {'nodes': state['comments'],
                        'pageInfo': {'hasNextPage': False, 'endCursor': None}}
            return {'data': {'viewer': {'login': VIEWER}, 'repository': {'pullRequest': {
                'headRefOid': state['head'], 'comments': comments}}}}

        def write_comment(self, state, comment, payload):
            comment['body'] = payload['body']
            path.write_text(json.dumps(state))
            self.reply({'node_id': comment['id'], 'html_url': comment['url'],
                        'body': comment['body'], 'user': {'login': VIEWER}})

        def reply(self, answer):
            body = json.dumps(answer).encode()
            self.send_response(200)
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
        else:
            path = arg
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
