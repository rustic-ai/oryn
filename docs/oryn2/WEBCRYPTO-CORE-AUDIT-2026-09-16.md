# Native WebCrypto Core dependency source audit — 2026-09-16

Status: implementation and regression sources generated; no build, tests, formatting, service execution, or WPT validation performed. Cargo.lock regeneration is deferred to the authorized post-generation build phase. This is a bounded Core dependency implementation, not full WebCrypto conformance.

## Inventory and resulting scope

`CORE-CRYPTO-SOURCE-INVENTORY-2026-09-16.json` records the pinned revision, 10,864 selected Core paths, literal crypto uses across Core roots/shared helper directories, source hashes and confirmed transitive edges. The secure webmessaging helper generates a nonextractable AES-CBC key and posts it to an insecure opener, which must receive messageerror. Signature-integrity authority tests transitively import an Ed25519 private JWK and sign signature bases. Content encoding tests digest up to 27,000,000 bytes with SHA-256 in Window and worker realms. The authentication workflow uses SHA-256 for PKCE.

Generated native operations: AES-CBC 128/192/256 generation, raw/JWK import/export, PKCS#7 encryption/decryption and wrapping/unwrapping; Ed25519 generation, private/public JWK, raw public/seed, SPKI/PKCS8 import/export and real signature generation/verification; SHA-1/256/384/512 digest; OS-backed integer-view randomness and UUID v4. RustCrypto AES/CBC, ed25519-dalek and SHA crates provide the actual primitives. The legacy authored crypto global shim is removed.

## Authority and transport

CryptoKey wrappers hold immutable native key records, not authored algorithm/extractable/usages authorities. Receiver identity is checked against native wrapper ownership. Descriptor mutation cannot change key operations. Key material uses zeroizing storage; Debug omits material. Native wrapper reclamation releases records and private V8 descriptor caches avoid permanent Global roots.

Typed WireHost::CryptoKey transports native slots with bounded material, version/algorithm/kind/usages validation. It is serializable, not transferable. Receiving realm authority must be secure; insecure decoding fails through the trusted native messageerror delivery path before exposing a key. Nonextractable keys clone with custody intact; export remains denied. Wire metadata validation rejects unsupported algorithms, inconsistent kinds, lengths and usages; this is validation of trusted native transport, not a cryptographic authentication layer for arbitrary external wire mutation.

Buffer inputs are copied before algorithm getters for digest/cipher/signature operations. importKey snapshots supported JWK members and raw bytes before algorithm normalization. Promise settlement uses a native microtask and preserves original getter exceptions. Cryptographic computation currently runs synchronously before settlement; there is no separate crypto task executor.

## Generated regressions

Native primitive sources cover AES known-answer ciphertext and padding failure, Ed25519 RFC8032 vectors and DER roundtrips, SHA vectors, nonextractable clone/export custody, redacted debug output, and malformed typed wire metadata/material.

V8 sources cover mutated descriptors, same-realm alias-preserving clones, transfer rejection, the exact Core private JWK, key pair extractability, AES-wrapped Ed25519 private keys, secure worker clone encryption/decryption, insecure trusted messageerror, random view bounds/identity, UUID format, original algorithm-getter exception identity and digest input snapshot timing.

## Limits and open coverage

Inputs are capped at 32 MiB; AES padded ciphertext stays within that bound. Random calls are capped at 65,536 bytes and active native key records at 4,096. Shared/resizable BufferSources are rejected. Key usages and JWK conversion are bounded. Required pinned digest bodies fit the input cap.

Unsupported algorithm families remain explicit: AES-GCM/CTR/KW, RSA, ECDSA/ECDH, HMAC, HKDF/PBKDF2, X25519/Ed448 and post-quantum algorithms. deriveBits/deriveKey reject NotSupportedError. The JsonWebKey IDL/conversion currently covers AES/OKP members, not unused RSA private fields. Full Necessary-tier WebCrypto, broader dictionary conversion/error ordering, scheduling behavior and resource-limit conformance remain open. Dependency scanning is literal source evidence plus confirmed helper inspection, not proof that every dynamically computed algorithm name is bounded.

All runtime and compilation claims await the combined post-generation validation phase.
