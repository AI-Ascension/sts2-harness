// SPDX-License-Identifier: MIT
// Check the bridge's static provider descriptor without contacting a provider.

import fs from 'node:fs';

const filename = process.argv[2];
if (!filename || process.argv.length !== 3) throw new Error('usage: node verify-describe.mjs DESCRIBE_JSON');
const value = JSON.parse(fs.readFileSync(filename, 'utf8'));
if (value.kind !== 'openai-astra' || value.provider !== 'openai' || value.model !== 'gpt-6-astra') {
  throw new Error('bridge descriptor does not identify the configured Astra provider boundary');
}
if (!value.map_profiles?.includes('runtime-map-v1') || !value.map_capabilities?.includes('graph') || !value.map_capabilities?.includes('graph-image')) {
  throw new Error('bridge descriptor omits the negotiated graph/image profile');
}
if (value.max_request_bytes !== 8_388_608 || value.media?.[0]?.media_type !== 'image/png') {
  throw new Error('bridge descriptor has unexpected request/image bounds');
}
