# Native page-worker sandbox

`oryn-page-worker` has no filesystem or network command in its protocol. On
macOS it is signed with the App Sandbox entitlement and communicates only over
stdin/stdout JSON lines.

Build and validate it with a locally installed Apple Development identity:

```bash
export ORYN_CODESIGN_IDENTITY="Apple Development: Example (TEAMID)"
./scripts/build-sandboxed-worker.sh
```

Ad-hoc signing is deliberately rejected. macOS can display the entitlement on
an ad-hoc-signed command-line executable but aborts it during sandbox
initialization because the signature has no trusted team identity. The G0
sandbox gate therefore requires a real development identity rather than a
cosmetic entitlement check.
