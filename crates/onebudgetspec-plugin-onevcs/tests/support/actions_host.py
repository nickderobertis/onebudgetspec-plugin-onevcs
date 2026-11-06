#!/usr/bin/env python3
"""GitHub as it answers the Actions token, over plain HTTP for the real `gh` to reach.

`serve STATE WIRE` answers from the STATE json file and prints its port once listening. Every
shape, identity and route comes from WIRE, the fixture record_github_wire.py recorded from
GitHub, so re-recording is what moves them. Each query it receives is kept in STATE for the
test that holds onevcs to the recorded one.
"""
import http.server
import json
from pathlib import Path
import sys
from urllib.parse import urlparse


def serve(path, wire):
    recorded = wire['actions_comment']
    rest = wire['actions_rest_comment']
    owner, name, _, number = urlparse(wire['source']['actions_change']).path.split('/')[1:]
    comments_route = urlparse(rest['issue_url']).path + '/comments'
    comment_route = urlparse(rest['url']).path.rsplit('/', 1)[0] + '/'

    class Host(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            payload = self.payload()
            route = urlparse(self.path).path
            state = json.loads(path.read_text())
            if route == '/graphql':
                return self.read_comments(state, payload)
            if route != comments_route:
                return self.refuse(404, 'no route: POST %s' % route)
            database_id = recorded['databaseId'] + len(state['comments'])
            comment = {**recorded, 'id': '%s-%d' % (recorded['id'], database_id),
                       'databaseId': database_id,
                       'url': '%s#issuecomment-%d' % (recorded['url'].split('#')[0], database_id)}
            state['comments'].append(comment)
            self.write_comment(state, comment, payload)

        def do_PATCH(self):
            payload = self.payload()
            route = urlparse(self.path).path
            state = json.loads(path.read_text())
            comment = next((c for c in state['comments']
                            if route == comment_route + str(c['databaseId'])), None)
            if comment is None:
                return self.refuse(404, 'no comment at: PATCH %s' % route)
            self.write_comment(state, comment, payload)

        def payload(self):
            payload = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            return payload if isinstance(payload, dict) else {}

        def read_comments(self, state, payload):
            expected = {'owner': owner, 'name': name, 'number': int(number)}
            if payload.get('variables') != expected:
                return self.refuse(422, 'unexpected variables: %s' % payload.get('variables'))
            state['queries'].append(payload.get('query'))
            path.write_text(json.dumps(state))
            self.reply({'data': {'viewer': {'login': rest['user']['login']}, 'repository': {
                'pullRequest': {'headRefOid': state['head'], 'comments': {
                    'nodes': state['comments'], 'pageInfo': wire['page_info']}}}}})

        def write_comment(self, state, comment, payload):
            if not isinstance(payload.get('body'), str):
                return self.refuse(422, 'body must be a string')
            comment['body'] = payload['body']
            path.write_text(json.dumps(state))
            written = {'id': comment['databaseId'], 'node_id': comment['id'],
                       'url': 'https://api.github.com%s%d' % (comment_route, comment['databaseId']),
                       'html_url': comment['url'], 'issue_url': rest['issue_url'],
                       'body': comment['body'], 'user': rest['user']}
            self.reply({field: written[field] for field in rest})

        def refuse(self, status, message):
            self.reply({'message': message}, status)

        def reply(self, answer, status=200):
            body = json.dumps(answer).encode()
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Host)
    print(server.server_port, flush=True)
    server.serve_forever()


if len(sys.argv) != 4 or sys.argv[1] != 'serve':
    sys.exit('usage: actions_host.py serve STATE WIRE')
serve(Path(sys.argv[2]), json.loads(Path(sys.argv[3]).read_text()))
