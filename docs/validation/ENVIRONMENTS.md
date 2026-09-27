# Offline WPT Python environment

`oryn_validation.environments` stages a regular Python executable, its complete
standard library, and the exact existing WPT dependency payload into the configured
data store. It does not install packages, resolve versions, download files, build
wheels, or run WPT. The only executed runtime check is a 120-second owned Python
identity/import probe with a 15-second cleanup grace period.

The source-owned `tools/validation/locks/wpt-python-3.14.json` pins Python 3.14.7
and the 40 third-party distributions used by the existing pinned WPT runner.
Every retained installed file, including distribution METADATA and WHEEL metadata,
has an exact digest, size and executable bit. Original dependency wheel archives
were not retained as migration inputs, so no archive digest or reconstruction is
claimed. Byte identities of the installed payload are authoritative for this
explicit offline migration. Updating the lock requires source review.

The old installed `oryn_wptrunner` package, its distribution metadata, bytecode
caches, and the system standard library's global `site-packages` entry are excluded.
A staged environment must not contain an installed Oryn plugin. Execution obtains
the current plugin from the separately pinned runner source export. Startup `.pth`
hooks, undeclared dependency files and payload symlinks fail admission.

## Preparation

The coordinator provides an exact regular system Python executable (resolve the
system installation's symlink when selecting it), its matching standard library,
and the existing dependency `site-packages` as one-time copy inputs. The CLI is:

```
python3 -B scripts/oryn-validate.py environment-prepare \
  --donor-site-packages <selected-installed-payload> \
  --system-python <regular-system-python3.14> \
  --stdlib <matching-system-stdlib> --store-root <data-root> \
  --environment-path environments/wpt-python \
  --identity-path identities/wpt-python.json \
  --diagnostics-path evidence/environment-probe-01
```

The underlying API is `prepare_environment(donor_site_packages, system_python,
stdlib, store_root, *, environment_path, identity_path, diagnostics_path,
lock_path=None)`. An optional lock path must have the exact source-owned bytes.
The three output paths must be fresh, separate store-relative paths. Failures
retain their original partial files and process evidence; choose new output paths
for a repaired attempt. Do not overwrite or publish a failed environment.

The returned assets are `python`, `python_environment`,
`python_environment_identity`, `python_dependency_lock`, and `probe`. All five belong in the run's explicit assets; the probe retains the owned process
proof and is required for admission. Verification
uses only staged bytes, the current source-owned dependency lock, original probe
receipts, and explicit system observations. It never opens the migration donor.

## Execution and verification

Call `verify_environment(identity, store_root)` during admission and final evidence
verification. It checks full staged membership, locked dependency payload,
interpreter/stdlib identity, original probe command and kernel-birth custody, and
system-library drift. Store relocation preserves the original probe paths as
historical evidence while runtime paths are derived from the new store root.

Every Python subprocess—including the coordinator, upstream server and spawned
server interpreters—must receive `runtime_environment(identity, store_root)`:

- `PYTHONHOME` is the staged environment directory.
- `PYTHONNOUSERSITE=1` prevents user-package loading.
- `PYTHONDONTWRITEBYTECODE=1` preserves immutable input bytes.

The WPT adapter additionally sets its exact runner-export `PYTHONPATH`; do not
inherit unrelated Python, virtual-environment or dynamic-loader overrides. Bind
the environment identity's interpreter, directory and lock to the same run assets.
A shell launcher does not satisfy the executable-file contract. Input admission
and final checks still cover the runner export and upstream independently.

Native extension dependencies are inspected with `otool`. Homebrew libraries
outside the store remain explicit selected/resolved path observations with byte
identities, checked transitively. macOS shared-cache libraries are recorded by
canonical system load name and macOS build; they are not represented as individually
copied or hash-certified files. Unexpected external roots and unresolved/ambiguous
load paths fail closed. This is relocatable store data on a matching observed macOS
system, not a fully self-contained OS distribution.

The independent review must preserve these system dependencies and probe limitations.
A successful environment check establishes Python runtime custody; it grants no
native, topology, canary, placement, full-population or release qualification.

## Developer regression check

```
PYTHONPATH=tools/validation PYTHONDONTWRITEBYTECODE=1 \
  python3 -m unittest discover -s tools/validation/tests -p test_environments.py -v
```

These tests stage tiny inert fixtures and mock process/system observation. They
exercise drift, membership, plugin exclusion, relocation, library classification,
and failed ownership evidence without running an interpreter installation or WPT.
