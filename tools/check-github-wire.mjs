// Holds the recorded GitHub fixture to GitHub's published contract, pinned in bun.lock, so
// upgrading those packages fails offline when GitHub's schema moves away from the recording.
// Usage: node tools/check-github-wire.mjs [WIRE] [GRAPHQL_SCHEMA] [REST_TYPES]
// Exit 0: the fixture conforms. Exit 1: stderr names what moved and how to re-record.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { GraphQLInt, buildSchema, execute, parse, validate } from 'graphql';

const packages = new URL('../node_modules/@octokit/', import.meta.url);
const [
  wirePath = new URL('../crates/onebudgetspec-plugin-onevcs/tests/fixtures/github-wire.json', import.meta.url),
  schemaPath = new URL('graphql-schema/schema.graphql', packages),
  restPath = new URL('openapi-types/types.d.ts', packages),
] = process.argv.slice(2);

const fail = (reason) => {
  console.error(`github wire: ${reason}; run \`just record-github-wire\` and review what moved.`);
  process.exit(1);
};

/** Each top-level field's declared type in one `"name": { ... }` schema of the REST types. */
function restFields(types, name) {
  const start = types.indexOf(`\n    "${name}": {\n`);
  if (start < 0) fail(`GitHub's REST types no longer declare ${name}`);
  const body = types.slice(start, types.indexOf('\n    };\n', start));
  return new Map([...body.matchAll(/^ {6}"?([a-z_]+)"?\??: (.+);$/gm)].map((match) => [match[1], match[2]]));
}

/** Whether a recorded JSON value is what a declared TypeScript union allows. */
function fits(value, declared) {
  return declared.split('|').map((type) => type.trim()).some((type) => {
    if (type === 'null') return value === null;
    if (type === 'bigint') return Number.isInteger(value);
    if (type.startsWith('components[')) return typeof value === 'object' && value !== null;
    return typeof value === type;
  });
}

function checkRest(recorded, fields, name, required) {
  for (const key of required) {
    if (!(key in recorded)) fail(`the recorded REST ${name} lacks ${key}`);
  }
  for (const [key, value] of Object.entries(recorded)) {
    if (!fields.has(key)) fail(`GitHub's REST ${name} does not declare ${key}`);
    if (!fits(value, fields.get(key))) {
      fail(`the recorded REST ${name} ${key} does not fit ${fields.get(key)}`);
    }
  }
}

// GitHub answers databaseId past the 32 bits its schema's Int declares (the fixture records one),
// so Int here means any whole number; anything else still fails.
GraphQLInt.serialize = (value) => {
  if (!Number.isSafeInteger(value)) throw new TypeError(`Int cannot represent ${value}`);
  return value;
};

let wire, schema, document, rest, data;
try {
  wire = JSON.parse(readFileSync(wirePath, 'utf8'));
  // The published SDL repeats two unrelated enterprise fields, so its own validity is not checked.
  schema = buildSchema(readFileSync(schemaPath, 'utf8'), { assumeValid: true, assumeValidSDL: true });
  document = parse(wire.query);
  rest = wire.actions_rest_comment;
  data = {
    viewer: { login: rest.user.login },
    repository: {
      pullRequest: {
        headRefOid: '0'.repeat(40),
        comments: { nodes: [wire.actions_comment, wire.person_comment], pageInfo: wire.page_info },
      },
    },
  };
} catch (error) {
  fail(`could not read the recording or the published schema: ${error.message}`);
}
const invalid = validate(schema, document);
if (invalid.length) fail(`the recorded query does not fit GitHub's schema: ${invalid[0].message}`);

const answered = await execute({
  schema,
  document,
  rootValue: data,
  variableValues: { owner: 'owner', name: 'name', number: 1 },
});
if (answered.errors?.length) {
  fail(`the recorded answers do not fit GitHub's schema: ${answered.errors[0].message}`);
}
try {
  assert.deepEqual(JSON.parse(JSON.stringify(answered.data)), data);
} catch {
  fail('the recorded answers do not round-trip through the recorded query');
}

const types = readFileSync(restPath, 'utf8');
const comment = restFields(types, 'issue-comment');
checkRest(rest, comment, 'issue comment', ['id', 'node_id', 'url', 'html_url', 'issue_url', 'body', 'user']);
const userSchema = /^components\["schemas"\]\["([a-z-]+)"\]$/.exec(comment.get('user'));
if (!userSchema) fail(`GitHub's REST issue comment user is no longer a schema: ${comment.get('user')}`);
checkRest(rest.user, restFields(types, userSchema[1]), 'user', ['login', 'type']);
