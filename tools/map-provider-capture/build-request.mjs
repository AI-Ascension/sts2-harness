// SPDX-License-Identifier: MIT
// Build one bounded sts2.exo-decision-map-v1 request from the checked-in graph fixture and the
// PNG emitted by the product renderer. This file deliberately has no provider or network code.

import crypto from 'node:crypto';
import fs from 'node:fs';

const MAX_REQUEST_BYTES = 8 * 1024 * 1024;
const MAX_IMAGE_BYTES = 8 * 1024 * 1024;
const MAX_IMAGE_WIDTH = 4096;
const MAX_IMAGE_HEIGHT = 4096;
const REQUIRED_SNAPSHOT_KEYS = [
  'state_id',
  'generation',
  'schema_version',
  'projection_version',
  'game_build',
  'mod_version',
  'map_instance_id',
  'act_id',
  'scope_id',
  'availability',
  'completeness',
  'freshness',
  'reason',
  'nodes',
  'edges',
  'position',
  'history',
  'terminal_node_ids',
  'bindings',
];

function fail(message) {
  throw new Error(message);
}

function parseArgs() {
  const values = new Map();
  const argv = process.argv.slice(2);
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    if (!key.startsWith('--') || index + 1 >= argv.length || argv[index + 1].startsWith('--')) {
      fail(`invalid argument: ${key}`);
    }
    if (values.has(key)) fail(`duplicate argument: ${key}`);
    values.set(key, argv[index + 1]);
    index += 1;
  }
  for (const key of ['--snapshot', '--png', '--rendered-manifest', '--out', '--metadata']) {
    if (!values.has(key)) fail(`missing argument: ${key}`);
  }
  return values;
}

function readJson(filename) {
  try {
    return JSON.parse(fs.readFileSync(filename, 'utf8'));
  } catch (error) {
    fail(`cannot read JSON artifact: ${error.message}`);
  }
}

function assertArray(value, name) {
  if (!Array.isArray(value)) fail(`${name} must be an array`);
  return value;
}

function assertString(value, name) {
  if (typeof value !== 'string' || value.length === 0) fail(`${name} must be a non-empty string`);
  return value;
}

function assertIdentity(value, name, maximum = 512) {
  assertString(value, name);
  if (value.length > maximum || !/^[A-Za-z0-9._:/-]+$/.test(value)) {
    fail(`${name} is not a bounded protocol identity`);
  }
  return value;
}

function compareStrings(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}

function normalizedSnapshot(snapshot) {
  const normalized = { ...snapshot };
  normalized.nodes = [...snapshot.nodes].sort((left, right) => compareStrings(left.id, right.id));
  normalized.edges = [...snapshot.edges].sort(
    (left, right) => compareStrings(left.from, right.from) || compareStrings(left.to, right.to),
  );
  normalized.terminal_node_ids = [...snapshot.terminal_node_ids].sort(compareStrings);
  normalized.bindings = [...snapshot.bindings].sort(
    (left, right) =>
      compareStrings(left.graph_node_id, right.graph_node_id) ||
      compareStrings(left.host_action_id, right.host_action_id),
  );
  return normalized;
}

function sha256(bytes) {
  return crypto.createHash('sha256').update(bytes).digest('hex');
}

function pngDimensions(bytes) {
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  if (bytes.length < 24 || !bytes.subarray(0, 8).equals(signature)) fail('renderer output is not a PNG');
  if (bytes.toString('ascii', 12, 16) !== 'IHDR') fail('PNG has no IHDR');
  const width = bytes.readUInt32BE(16);
  const height = bytes.readUInt32BE(20);
  if (width === 0 || height === 0 || width > MAX_IMAGE_WIDTH || height > MAX_IMAGE_HEIGHT) {
    fail('renderer PNG dimensions exceed the negotiated bound');
  }
  return { width, height };
}

function validateSnapshot(snapshot) {
  const keys = Object.keys(snapshot);
  if (
    keys.length !== REQUIRED_SNAPSHOT_KEYS.length ||
    REQUIRED_SNAPSHOT_KEYS.some((key) => !Object.prototype.hasOwnProperty.call(snapshot, key))
  ) {
    fail('fixture snapshot does not match visible-map-v1 field set');
  }
  if (snapshot.schema_version !== 'visible-map-v1' || snapshot.projection_version !== 'runtime-map-v1') {
    fail('fixture snapshot has the wrong map schema');
  }
  if (snapshot.availability !== 'available' || snapshot.completeness !== 'complete' || snapshot.freshness !== 'current') {
    fail('fixture snapshot is not complete and current');
  }
  if (snapshot.reason !== null) fail('fixture snapshot reason must be null');
  assertIdentity(snapshot.state_id, 'snapshot.state_id', 128);
  if (!Number.isSafeInteger(snapshot.generation)) fail('snapshot.generation must be a safe integer');
  if (!snapshot.position || typeof snapshot.position !== 'object') fail('snapshot.position is missing');
  assertString(snapshot.position.node_id, 'snapshot.position.node_id');

  const nodes = assertArray(snapshot.nodes, 'snapshot.nodes');
  const edges = assertArray(snapshot.edges, 'snapshot.edges');
  const bindings = assertArray(snapshot.bindings, 'snapshot.bindings');
  const terminals = assertArray(snapshot.terminal_node_ids, 'snapshot.terminal_node_ids');
  if (nodes.length !== 76 || edges.length !== 182) {
    fail(`capture requires the full demo graph (got ${nodes.length} nodes and ${edges.length} edges)`);
  }
  if (nodes.length > 256 || edges.length > 1024 || bindings.length > 256) {
    fail('fixture graph exceeds the provider boundary bounds');
  }

  const nodeIds = new Set();
  for (const node of nodes) {
    if (!node || typeof node !== 'object') fail('snapshot node is not an object');
    const id = assertIdentity(node.id, 'snapshot node id', 128);
    if (nodeIds.has(id)) fail(`duplicate snapshot node: ${id}`);
    nodeIds.add(id);
    for (const key of ['row', 'column', 'category', 'visited']) {
      if (!Object.prototype.hasOwnProperty.call(node, key)) fail(`snapshot node lacks ${key}`);
    }
  }

  const edgeIds = new Set();
  for (const edge of edges) {
    if (!edge || typeof edge !== 'object') fail('snapshot edge is not an object');
    const from = assertIdentity(edge.from, 'snapshot edge.from', 128);
    const to = assertIdentity(edge.to, 'snapshot edge.to', 128);
    if (!nodeIds.has(from) || !nodeIds.has(to)) fail('snapshot edge endpoint is not a node');
    const identity = `${from}\u0000${to}`;
    if (edgeIds.has(identity)) fail('snapshot contains a duplicate edge');
    edgeIds.add(identity);
  }
  for (const terminal of terminals) {
    if (!nodeIds.has(assertString(terminal, 'terminal node id'))) fail('terminal is not a graph node');
  }

  const actionIds = [];
  const optionIds = new Set();
  for (const binding of bindings) {
    if (!binding || typeof binding !== 'object') fail('snapshot binding is not an object');
    const graphNodeId = assertIdentity(binding.graph_node_id, 'binding.graph_node_id', 128);
    const hostActionId = assertIdentity(binding.host_action_id, 'binding.host_action_id', 512);
    if (!nodeIds.has(graphNodeId)) fail('binding graph node is not in the graph');
    if (!binding.action || binding.action.kind !== 'select_map_node') fail('binding action is not select_map_node');
    const optionId = assertIdentity(binding.action.node_id, 'binding.action.node_id', 128);
    if (optionIds.has(optionId)) fail('snapshot contains duplicate action option IDs');
    optionIds.add(optionId);
    if (actionIds.includes(hostActionId)) fail('snapshot contains duplicate host action IDs');
    actionIds.push(hostActionId);
  }
  if (bindings.length === 0 || bindings.length > 256) fail('snapshot has no bounded legal map bindings');
  return { nodes, edges, bindings, nodeIds, actionIds };
}

function buildObservation(snapshot, bindings) {
  return {
    state_id: snapshot.state_id,
    generation: snapshot.generation,
    player: {
      hp: 70,
      max_hp: 70,
      energy: 3,
      gold: 99,
      hand: [],
      deck: [],
      discard: [],
      exhaust: [],
    },
    state: {
      state: 'map',
      node_id: snapshot.position.node_id,
      options: bindings.map((binding) => binding.action.node_id),
    },
    legal_actions: bindings.map((binding) => ({
      action_id: binding.host_action_id,
      action: {
        kind: 'select_map_node',
        node_id: binding.action.node_id,
      },
    })),
  };
}

const args = parseArgs();
const snapshot = readJson(args.get('--snapshot'));
const { nodes, edges, bindings, nodeIds, actionIds } = validateSnapshot(snapshot);
const png = fs.readFileSync(args.get('--png'));
if (png.length === 0 || png.length > MAX_IMAGE_BYTES) fail('renderer PNG is outside its byte bound');
const { width, height } = pngDimensions(png);
const renderedManifest = readJson(args.get('--rendered-manifest'));
const graphDigest = sha256(Buffer.from(JSON.stringify(normalizedSnapshot(snapshot))));
const pngDigest = sha256(png);
const fixtureSnapshotDigest = sha256(fs.readFileSync(args.get('--snapshot')));
if (renderedManifest.snapshot_digest !== fixtureSnapshotDigest) fail('renderer manifest snapshot bytes do not match fixture');
if (renderedManifest.contents?.png_digest !== pngDigest) fail('renderer manifest PNG digest does not match output');
if (renderedManifest.presentation?.width !== width || renderedManifest.presentation?.height !== height) {
  fail('renderer manifest dimensions do not match output PNG');
}

const providerRevision = args.get('--provider-revision') ?? '5b1d196480685a313bc2417d5b4450a63cc89ce5';
if (!/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(providerRevision) || /^0+$/.test(providerRevision)) {
  fail('provider revision must be a non-zero lowercase Git-style digest');
}
const modelExecutionId = args.get('--model-execution-id') ?? 'capture-model-execution-001';
assertIdentity(modelExecutionId, 'model_execution_id');
const capability = {
  profile: 'runtime-map-v1',
  fair_play_version: 'fair-play-v1',
  graph: true,
  max_request_bytes: MAX_REQUEST_BYTES,
  image_png: {
    media_type: 'image/png',
    max_bytes: MAX_IMAGE_BYTES,
    max_width: MAX_IMAGE_WIDTH,
    max_height: MAX_IMAGE_HEIGHT,
  },
};
const image = {
  media_type: 'image/png',
  bytes_base64: png.toString('base64'),
  sha256: pngDigest,
  graph_digest: graphDigest,
  width,
  height,
};
const observation = buildObservation(snapshot, bindings);
const request = {
  schema: 'sts2.exo-decision-map-v1',
  provider_revision: providerRevision,
  model_execution_id: modelExecutionId,
  state_id: snapshot.state_id,
  generation: snapshot.generation,
  observation,
  legal_action_ids: actionIds,
  objective: 'Choose the first currently available map destination.',
  hard_constraints: [
    'Return only legal supplied action IDs.',
    'Use visible map data only.',
    'Stop after one action.',
  ],
  max_response_bytes: 8192,
  map_context: {
    capability,
    snapshot,
    image,
  },
};
const encoded = Buffer.from(JSON.stringify(request));
if (encoded.length > MAX_REQUEST_BYTES) fail('serialized Exo request exceeds its negotiated bound');

const metadata = {
  schema: request.schema,
  provider_revision: providerRevision,
  model_execution_id: modelExecutionId,
  state_id: snapshot.state_id,
  generation: snapshot.generation,
  graph_digest: graphDigest,
  fixture_snapshot_sha256: fixtureSnapshotDigest,
  png_sha256: pngDigest,
  png_bytes: png.length,
  width,
  height,
  nodes: nodes.length,
  edges: edges.length,
  bindings: bindings.length,
  request_bytes: encoded.length,
  fixture_bundle_digest: renderedManifest.bundle_digest ?? null,
  renderer_snapshot_digest: renderedManifest.snapshot_digest,
  renderer_png_digest: renderedManifest.contents?.png_digest ?? null,
};
fs.writeFileSync(args.get('--out'), `${JSON.stringify(request)}\n`, { flag: 'w' });
fs.writeFileSync(args.get('--metadata'), `${JSON.stringify(metadata, null, 2)}\n`, { flag: 'w' });
