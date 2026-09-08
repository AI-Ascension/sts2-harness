# Worker bootstrap v1 consumer artifacts

Owner: AI-Ascension/ascension-watchdog. These MIT-licensed, original synthetic
artifacts are copied unchanged from revision
`9fcff80b77ef2277a73abce38cb11eb4617d0458`, under
`schemas/worker-bootstrap-v1/`. They contain invented identities, not credentials.

SHA-256 of exact UTF-8 bytes:

- `schema.json`: `2a5321b37e5d2dd581d579cfbf7f6f00a37d13af624e21cbd409d9712a519d9d`
- `valid/linux.json`: `d2b382ebb5c1525400c53486562ad27947aedd052608f59d81152f9dd106fcd8`
- `valid/windows.json`: `6034ef4ecc46d149b838d3a468cc7065546fb8f6450122675b41c81d748cab57`

The harness independently implements the bounded decoder. Parsing expected peer
policy does not authenticate an OS peer or establish gameplay authority. The
native startup transport and actual peer verification remain separate gates.
