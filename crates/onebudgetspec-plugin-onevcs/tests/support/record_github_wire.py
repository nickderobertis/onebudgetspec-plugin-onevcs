#!/usr/bin/env python3
"""Record tests/fixtures/github-wire.json from GitHub, read-only, with the real plugin and gh.

`--plugin BIN --output PATH --actions-change URL --person-change URL` runs the plugin once per
change, with this script standing in as `gh`. The plugin picks the query, real `gh` answers it,
and the stated head never matches, so onevcs writes nothing; a write would be refused here too.
Comment bodies are elided: only the wire shape and identities are kept.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

RECORDING = 'RECORD_GITHUB_WIRE_READS'
UNMATCHED_HEAD = '0' * 40
CHANGE = re.compile(r'https://github\.com/([\w.-]+)/([\w.-]+)/pull/[1-9][0-9]*')
REST_FIELDS = ('id', 'node_id', 'url', 'html_url', 'issue_url')
ACTIONS_AUTHOR = {'__typename': 'Bot', 'login': 'github-actions'}
ACTIONS_REST_USER = {'login': 'github-actions[bot]', 'type': 'Bot'}


def stand_in_for_gh(args, reads):
    fields = args[2:]
    query = next((a.split('=', 1)[1] for a in fields if a.startswith('query=')), '')
    # Only `api graphql` and its field flags pass, so no spelling of a method override can.
    if (args[:2] != ['api', 'graphql'] or len(fields) % 2 or set(fields[::2]) - {'-f', '-F'}
            or not query.startswith('query') or re.search(r'\bmutation\b', query)):
        sys.exit('recording is read-only; refused: gh %s' % ' '.join(args[:2]))
    answer = subprocess.run(['gh'] + args, stdout=subprocess.PIPE, check=False)
    if answer.returncode:
        sys.exit(answer.returncode)
    with open(reads, 'a') as log:
        log.write(json.dumps({'query': query, 'answer': json.loads(answer.stdout)}) + '\n')
    sys.stdout.buffer.write(answer.stdout)


def read_through_plugin(plugin, change):
    with tempfile.TemporaryDirectory() as scratch:
        reads = Path(scratch) / 'reads.jsonl'
        request = {'schema_version': 1, 'destination': change, 'key': 'github-wire-recording',
                   'head_sha': UNMATCHED_HEAD, 'run_id': '1', 'run_url': change,
                   'markdown': 'recording', 'attempt': 1, 'flags': []}
        answer = subprocess.run([plugin], input=json.dumps(request).encode(),
                                env={**os.environ, 'ONEVCS_GH': str(Path(__file__).resolve()),
                                     RECORDING: str(reads)},
                                stdout=subprocess.PIPE, check=False)
        result = json.loads(answer.stdout or '{}')
        # Every page read and the head refused is the only way this run can finish well.
        if (answer.returncode or result.get('delivered') is not False or not reads.exists()
                or 'expected head' not in (result.get('diagnostic') or '')):
            sys.exit('could not read %s through the plugin: %s; check gh auth status and retry.'
                     % (change, result.get('diagnostic') or answer.stdout.decode()))
        return [json.loads(line) for line in reads.read_text().splitlines()]


def first_comment(reads, wanted, change):
    for read in reads:
        comments = read['answer']['data']['repository']['pullRequest']['comments']
        for node in comments['nodes']:
            author = node.get('author') or {}
            if all(author.get(key) == value for key, value in wanted.items()):
                return {**node, 'body': ''}, comments['pageInfo']
    sys.exit('%s holds no comment by %s; pass a change that does.' % (change, wanted))


def rest_comment(owner, name, comment):
    answer = subprocess.run(
        ['gh', 'api', 'repos/%s/%s/issues/comments/%d' % (owner, name, comment['databaseId'])],
        stdout=subprocess.PIPE, check=False)
    if answer.returncode:
        sys.exit('could not read comment %d over REST; check gh auth status and retry.'
                 % comment['databaseId'])
    rest = json.loads(answer.stdout)
    user = {key: rest.get('user', {}).get(key) for key in ACTIONS_REST_USER}
    if (any(field not in rest for field in REST_FIELDS) or rest['id'] != comment['databaseId']
            or rest['node_id'] != comment['id'] or user != ACTIONS_REST_USER):
        sys.exit('the REST answer for comment %d is not that Actions comment: %s'
                 % (comment['databaseId'], sorted(rest)))
    return {**{field: rest[field] for field in REST_FIELDS}, 'body': '', 'user': user}


def record(arguments):
    actions_reads = read_through_plugin(arguments.plugin, arguments.actions_change)
    actions, page_info = first_comment(actions_reads, ACTIONS_AUTHOR, arguments.actions_change)
    person, _ = first_comment(read_through_plugin(arguments.plugin, arguments.person_change),
                              {'__typename': 'User'}, arguments.person_change)
    owner, name = CHANGE.fullmatch(arguments.actions_change).groups()
    rest = rest_comment(owner, name, actions)
    arguments.output.write_text(json.dumps({
        'source': {'actions_change': arguments.actions_change,
                   'person_change': arguments.person_change},
        'query': actions_reads[0]['query'],
        'page_info': {**page_info, 'hasNextPage': False, 'endCursor': None},
        'actions_comment': actions,
        'person_comment': person,
        'actions_rest_comment': rest,
    }, indent=2, sort_keys=True) + '\n')


def change_url(text):
    if not CHANGE.fullmatch(text):
        raise argparse.ArgumentTypeError('expected https://github.com/OWNER/NAME/pull/NUMBER')
    return text


if RECORDING in os.environ:
    stand_in_for_gh(sys.argv[1:], os.environ[RECORDING])
else:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plugin', required=True)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--actions-change', required=True, type=change_url)
    parser.add_argument('--person-change', required=True, type=change_url)
    record(parser.parse_args())
