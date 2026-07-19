# builder-sign-rs — open-ot builder signing CLI

Rust implementation of the SPEC §9 build → sign → pin pipeline:

```
cargo build --target wasm32-wasi   →   wamrc --enable-aot   →
builder-sign sign cell.aot         →   blake3 CID + Ed25519 signature   →
XRPC com.etzhayyim.apps.openOt.pinModule →   atproto record links cell DID → CID + sig
```

Replaces the `scripts/builder-sign.sh` stub referenced in `cells/CLAUDE.md` (removed 2026-05-20). Per Gate C report §2.5.

## Status

`v0.1.0` — Tier 1 deliverable for Gate C follow-up (per `risk1/gate-c-estimate/gate-c-report.md`). Working CLI + library + 10 roundtrip tests. Not yet wired into the cell build pipeline; that wiring happens at Mimi Rev-1 firmware spin.

## Crypto

- Hash: **BLAKE3** (b3sum-compatible, 256-bit) — content addressing for AOT artefacts. Matches the SPEC §9 `b3sum` recipe.
- Signature: **Ed25519** — builder DID's Ed25519 key signs the BLAKE3 hash. Verified on Mimi/Te edge devices at boot per SPEC §9 step 5.
- Encoding: **base58btc** for CIDs (compact, URL-safe-ish, well-supported); **hex** for signatures + private keys (standard for raw bytes).

These primitives are NOT FIPS-listed; if a deployment requires FIPS-140-3 (e.g. US-government infra), swap to RSA-3072 + SHA-256 per gate-c-report.md §4. Estimated 1.5 PM of work, not counted in the 4.75 PM Gate C estimate.

## CLI

```
builder-sign <SUBCOMMAND>

Subcommands:
  keygen       Generate a fresh Ed25519 keypair and print to stdout
  sign         Sign a file: emit BLAKE3 CID + Ed25519 signature
  verify       Verify a signature against a file and public key
  cid          Print just the BLAKE3 CID for a file (no signing)
```

Each subcommand has its own `--help`.

### keygen

```bash
$ builder-sign keygen --out builder.key
[builder-sign] wrote private key to builder.key (32 bytes hex, mode 0600)
[builder-sign] public key (hex):    0c4f...e1
[builder-sign] DID-suffix candidate: z6Mk... (multibase ed25519-pub)
```

### sign

```bash
$ builder-sign sign \
    --input cells/target/wasm32-unknown-unknown/release/pid_limited.wasm \
    --key   builder.key

cid_blake3:  zN4mU7e...   (base58btc-encoded 32-byte BLAKE3 hash)
sig_ed25519: 4a3f...      (hex-encoded 64-byte Ed25519 signature over the BLAKE3 hash)
size_bytes:  2389
```

The signature is computed over the **BLAKE3 hash**, not the raw file — this matches the SPEC §9 pinModule contract where the on-chain record stores both `moduleCid` and `moduleSig`, and verification only needs the hash for replay defence.

### verify

```bash
$ builder-sign verify \
    --input cells/target/wasm32-unknown-unknown/release/pid_limited.wasm \
    --signature 4a3f... \
    --public-key 0c4f...
[builder-sign] cid_blake3:  zN4mU7e...
[builder-sign] signature:   VALID
```

Exit code 0 on valid signature, 1 on invalid (so it composes with shell pipelines / CI gates).

### cid

```bash
$ builder-sign cid --input cells/target/wasm32-unknown-unknown/release/pid_limited.wasm
zN4mU7e...
```

For pre-flight content-addressing without signing.

## Library API

```rust
use builder_sign_rs::{hash_file, sign_hash, verify_hash, Keypair, B3Cid};

let cid: B3Cid = hash_file(path)?;          // BLAKE3 over file contents
let signature = sign_hash(&cid, &keypair);  // Ed25519 over the 32-byte hash
let ok = verify_hash(&cid, &signature, &keypair.public_key());
```

The library is what gets wired into the build script + the on-device verify shim
(`firmware/mimi-zephyr/src/aot-verify.c` — future work in §2.5 of Gate C report).

## Tests

11 unit + integration tests under `src/lib.rs` `#[cfg(test)] mod tests`:

- `keygen_roundtrip` — generate, serialise hex, parse, regenerate; bytes equal
- `sign_verify_roundtrip` — sign a known payload, verify; ok
- `sign_verify_tampered_payload` — sign, mutate payload, verify; rejected
- `sign_verify_tampered_signature` — sign, flip a signature bit, verify; rejected
- `sign_verify_wrong_pubkey` — sign with key A, verify with key B; rejected
- `cid_known_vector` — BLAKE3 of empty input matches the standard known vector
- `cid_deterministic` — same file → same CID across two runs
- `cid_distinct` — different file content → different CIDs
- `cid_base58_roundtrip` — to_string → parse → bytes equal
- `signature_serialise_roundtrip` — hex serialise + parse + verify; still valid
- `public_key_serialise_roundtrip` — hex serialise + parse; bytes equal

Run with `cargo test`.

## Security notes

- Private keys are loaded from file path or stdin. **Never from CLI args** (which would leak to process listings). `keygen` writes with mode `0600`.
- BLAKE3 is collision-resistant under standard assumptions; 256-bit output is adequate for build artefacts.
- Ed25519 verification on Cortex-M7 (Mimi/Te) benchmarks at ~5 ms via the embedded ed25519 path used by MCUboot — well within boot-time budget per Gate C §2.5.
