// SPDX-License-Identifier: MIT
// Verify evidence captured at the actual sts2-astra-bridge -> codex CLI boundary. The verifier
// compares the bridge's prompt projection and image transport to the serialized request and never
// prints or writes the private prompt itself.

import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

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
  for (const key of [
    '--request',
    '--prompt',
    '--image',
    '--args',
    '--decision',
    '--bridge-output',
    '--render-json',
    '--metadata',
    '--count',
    '--source-png',
    '--bridge-sha256',
    '--renderer-sha256',
    '--report',
  ]) {
    if (!values.has(key)) fail(`missing argument: ${key}`);
  }
  return values;
}

function readJson(filename) {
  try {
    return JSON.parse(fs.readFileSync(filename, 'utf8'));
  } catch (error) {
    fail(`invalid JSON evidence: ${error.message}`);
  }
}

function readBytes(filename) {
  try {
    return fs.readFileSync(filename);
  } catch (error) {
    fail(`missing capture evidence: ${error.message}`);
  }
}

function sha256(bytes) {
  return crypto.createHash('sha256').update(bytes).digest('hex');
}

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]));
  }
  return value;
}

function equalJson(left, right) {
  return JSON.stringify(canonical(left)) === JSON.stringify(canonical(right));
}

function parsePrompt(promptBytes) {
  const prompt = promptBytes.toString('utf8');
  const marker = prompt.lastIndexOf('\n{');
  if (marker < 0) fail('captured provider prompt has no serialized request');
  let request;
  try {
    request = JSON.parse(prompt.slice(marker + 1));
  } catch (error) {
    fail(`captured provider prompt request is invalid JSON: ${error.message}`);
  }
  if (prompt.includes('"bytes_base64"')) fail('provider prompt retained map image bytes');
  return request;
}

function parseNulArguments(bytes) {
  const values = bytes.toString('utf8').split('\0');
  if (values.at(-1) === '') values.pop();
  if (values.length === 0 || values.some((value) => value.length === 0)) fail('CLI argument capture is empty or malformed');
  return values;
}

function requireArguments(args, request, capturedImagePath) {
  const prefix = [
    'exec',
    '--ignore-user-config',
    '--ephemeral',
    '--skip-git-repo-check',
    '--sandbox',
    'read-only',
    '--disable',
    'shell_tool',
    '--disable',
    'multi_agent',
    '--disable',
    'apps',
    '--disable',
    'in_app_browser',
    '--disable',
    'in_app_local_automation',
    '--disable',
    'sleep_tool',
    '-c',
    'web_search="disabled"',
    '-c',
    'project_doc_max_bytes=0',
    '-c',
    'model_reasoning_effort="low"',
    '-m',
    'gpt-6-astra',
    '--color',
    'never',
    '--cd',
  ];
  if (prefix.some((value, index) => args[index] !== value)) fail('bridge provider CLI arguments changed unexpectedly');
  let cursor = prefix.length;
  const cdPath = args[cursor];
  if (!cdPath) fail('bridge provider CLI omitted --cd value');
  cursor += 1;
  if (args[cursor] !== '--output-schema' || !args[cursor + 1]) fail('bridge provider CLI omitted output schema');
  const schemaPath = args[cursor + 1];
  cursor += 2;
  if (args[cursor] !== '--output-last-message' || !args[cursor + 1]) fail('bridge provider CLI omitted output file');
  const outputPath = args[cursor + 1];
  cursor += 2;
  if (args[cursor] !== '--image' || !args[cursor + 1]) fail('bridge provider CLI omitted map image');
  if (args[cursor + 1] !== capturedImagePath) fail('captured image path differs from --image argument');
  cursor += 2;
  if (args[cursor] !== '-' || cursor + 1 !== args.length) fail('bridge provider CLI did not use stdin-only prompt input');
  if (path.dirname(schemaPath) !== path.dirname(outputPath) || path.dirname(outputPath) !== cdPath) {
    fail('bridge provider CLI temporary paths are not isolated to one workspace');
  }
  if (request.model_execution_id.length === 0) fail('request execution ID is empty');
  return { cdPath, schemaPath, outputPath };
}

function assertGraphIdentity(request, prompt) {
  const requestSnapshot = request.map_context?.snapshot;
  const promptSnapshot = prompt.map_context?.snapshot;
  if (!requestSnapshot || !promptSnapshot) fail('map snapshot is absent at CLI boundary');
  const requestNodes = requestSnapshot.nodes;
  const requestEdges = requestSnapshot.edges;
  if (!Array.isArray(requestNodes) || !Array.isArray(requestEdges)) fail('map graph arrays are absent');
  if (requestNodes.length !== 76 || requestEdges.length !== 182) fail('captured request is not the full graph');
  if (!equalJson(requestSnapshot, promptSnapshot)) fail('provider prompt graph differs from serialized request');
  const nodeIds = new Set(requestNodes.map((node) => node.id));
  const edgeIds = new Set();
  for (const edge of requestEdges) {
    if (!nodeIds.has(edge.from) || !nodeIds.has(edge.to)) fail('captured graph has an unknown edge endpoint');
    const edgeId = `${edge.from}\u0000${edge.to}`;
    if (edgeIds.has(edgeId)) fail('captured graph has a duplicate edge');
    edgeIds.add(edgeId);
  }
  return { nodes: requestNodes.length, edges: requestEdges.length, bindings: requestSnapshot.bindings.length };
}

function writeReport(reportPath, metadata, render, request, prompt, image, decision, bridgeOutput, args, bridgeSha, rendererSha, graph) {
  const imageSha = sha256(image);
  const promptSha = sha256(prompt);
  const argsSha = sha256(args);
  const decisionSha = sha256(decision);
  const bridgeOutputSha = sha256(bridgeOutput);
  const lines = [
    '# Actual Astra provider CLI capture',
    '',
    'Status: **confirmed for this bounded local boundary test**.',
    '',
    'This record covers one serialized `sts2.exo-decision-map-v1` request through the built `sts2-astra-bridge`, the actual pinned product renderer, and a private `PATH` shim named `codex`. The shim ran once, captured the prompt, image bytes, and argv, and wrote the bounded decision file requested by the bridge. No real provider process or network call was used.',
    '',
    '## Source and artifact pins',
    '',
    `- Product renderer: SHA-256 \`${rendererSha}\`; build commit \`5b1d196480685a313bc2417d5b4450a63cc89ce5\`.`,
    `- Astra bridge executable: SHA-256 \`${bridgeSha}\`.`,
    `- Provider revision in request: \`${metadata.provider_revision}\`.`,
    `- Request schema: \`${metadata.schema}\`; serialized size: ${metadata.request_bytes} bytes.`,
    '- Bridge preflight: `sts2-astra-bridge --describe`; renderer preflight: `map-visualizer validate --bundle <full-demo-bundle>`, followed by `render --width 1600 --height 2400`.',
    '- Finite timeouts: bridge descriptor 5 seconds, renderer validation/render 20 seconds each, outer bridge capture 20 seconds; the bridge provider subprocess is bounded by its configured 90-second timeout.',
    '',
    '## Graph and image evidence',
    '',
    `- Full graph verified at request and prompt boundaries: ${graph.nodes} nodes, ${graph.edges} directed edges, ${graph.bindings} legal bindings; every edge endpoint resolves to a captured node and no duplicate edge was accepted.`,
    `- Provider graph digest (normalized visible-map-v1): \`${metadata.graph_digest}\`; renderer/fixture snapshot bytes: \`${metadata.fixture_snapshot_sha256}\`. The two digests are recorded separately because the provider normalizes graph arrays while the renderer manifest hashes source snapshot bytes.`,
    `- Renderer output: ${metadata.width}×${metadata.height}, ${metadata.png_bytes} bytes, SHA-256 \`${metadata.png_sha256}\`; captured \`--image\` bytes: SHA-256 \`${imageSha}\`.`,
    `- Renderer JSON PNG digest: \`${render.png_digest}\`; captured image and rendered PNG are byte-identical.`,
    '',
    '## Execution identity and CLI evidence',
    '',
    `- \`model_execution_id\`: \`${metadata.model_execution_id}\`; \`state_id\`: \`${metadata.state_id}\`; \`generation\`: ${metadata.generation}.`,
    `- Captured prompt SHA-256: \`${promptSha}\`; the prompt contains the request identities and graph but no \`bytes_base64\` field.`,
    `- Captured argv SHA-256: \`${argsSha}\`; the verifier checked the bridge’s exact sandbox, disabled-tool, model, output-file, image, and stdin flags.`,
    `- Shim decision bytes: ${decision.length}, SHA-256 \`${decisionSha}\`; bridge output SHA-256: \`${bridgeOutputSha}\`; both selected the first supplied legal action and passed bridge validation.`,
    '',
    '## Reproduction',
    '',
    '```text',
    './tools/map-provider-capture/capture.sh \\',
    '  --bridge <built-sts2-astra-bridge> \\',
    '  --renderer <map-visualizer-at-pinned-sha> \\',
    '  --fixture <map-bundle-demo-v1> \\',
    '  --report .orchestration/provider-cli-capture.md',
    '```',
    '',
    'The harness worktree was not used to launch a game, MCP server, gateway, account, or real provider. Those boundaries remain outside this evidence.',
    '',
  ];
  fs.mkdirSync(path.dirname(reportPath), { recursive: true });
  fs.writeFileSync(reportPath, lines.join('\n'), { flag: 'w' });
}

const args = parseArgs();
const request = readJson(args.get('--request'));
const metadata = readJson(args.get('--metadata'));
const render = readJson(args.get('--render-json'));
const prompt = readBytes(args.get('--prompt'));
const capturedImage = readBytes(args.get('--image'));
const cliArgsBytes = readBytes(args.get('--args'));
const decision = readBytes(args.get('--decision'));
const bridgeOutputBytes = readBytes(args.get('--bridge-output'));
if (fs.readFileSync(args.get('--count'), 'utf8') !== '1') fail('fake codex invocation count was not exactly one');
const promptRequest = parsePrompt(prompt);
const promptWithoutImage = structuredClone(request);
delete promptWithoutImage.map_context.image.bytes_base64;
if (!equalJson(promptWithoutImage, promptRequest)) fail('bridge prompt projection differs from request');
if (request.model_execution_id !== promptRequest.model_execution_id) fail('model execution ID was not carried to CLI prompt');
if (request.state_id !== promptRequest.state_id || request.generation !== promptRequest.generation) fail('state identity was not carried to CLI prompt');
if (sha256(capturedImage) !== request.map_context.image.sha256) fail('captured --image bytes have the wrong digest');
if (!capturedImage.equals(readBytes(args.get('--source-png')))) fail('captured --image bytes differ from renderer output');
const cliArgs = parseNulArguments(cliArgsBytes);
const imageArgument = cliArgs[cliArgs.indexOf('--image') + 1];
if (!imageArgument) fail('captured CLI argv has no image path');
requireArguments(cliArgs, request, imageArgument);
const graph = assertGraphIdentity(request, promptRequest);
if (decision.length === 0 || decision.length > request.max_response_bytes) fail('shim decision exceeded request response bound');
const decisionValue = readJson(args.get('--decision'));
if (!equalJson(decisionValue, { action_ids: [request.legal_action_ids[0]], rationale: 'bounded fake capture' })) {
  fail('shim did not write the bounded first-legal-action decision');
}
const bridgeOutput = JSON.parse(bridgeOutputBytes.toString('utf8'));
if (!equalJson(bridgeOutput, {
  decision: 'plan',
  action_ids: [request.legal_action_ids[0]],
  rationale: 'bounded fake capture',
})) fail('bridge output was not the validated shim decision');
if (render.png_digest !== request.map_context.image.sha256) fail('renderer report digest differs from request');
if (render.png_bytes !== capturedImage.length) fail('renderer report byte count differs from PNG output');
if (render.width !== request.map_context.image.width || render.height !== request.map_context.image.height) fail('renderer dimensions differ from request');
writeReport(
  args.get('--report'),
  metadata,
  render,
  request,
  prompt,
  capturedImage,
  decision,
  bridgeOutputBytes,
  cliArgsBytes,
  args.get('--bridge-sha256'),
  args.get('--renderer-sha256'),
  graph,
);
process.stdout.write(JSON.stringify({
  status: 'confirmed',
  nodes: graph.nodes,
  edges: graph.edges,
  bindings: graph.bindings,
  graph_digest: metadata.graph_digest,
  png_sha256: request.map_context.image.sha256,
  model_execution_id: request.model_execution_id,
  state_id: request.state_id,
  generation: request.generation,
}) + '\n');
