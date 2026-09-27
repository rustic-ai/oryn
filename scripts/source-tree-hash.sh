#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# Keep Git's display-path format: switching to -z would change existing identities
# for quoted filenames. Hash ordinary inventories in one Python process instead
# of launching shasum and awk for every file.
exec python3 - <<'PY'
import hashlib
import os
import subprocess
import sys

EXCLUDED = {b'docs/oryn2/IMPLEMENTATION_STATUS.md',
            b'docs/oryn2/CAPABILITY_MILESTONES.md'}


def included(path):
    return (path not in EXCLUDED
            and not path.startswith(b'benchmarks/evidence/')
            and os.path.isfile(path))


# Byte ordering is LC_ALL=C sort; retain duplicates and Git quoting verbatim.
paths = sorted(subprocess.check_output(
    ['git', 'ls-files', '--cached', '--others', '--exclude-standard']).split(b'\n'))

# The historical shasum invocation has no -- delimiter: an eligible leading-dash
# filename is interpreted as an option (or '-' consumes the inventory pipe).
# Preserve that exceptional behavior instead of silently changing old hashes.
if any(path.startswith(b'-') and included(path) for path in paths):
    os.execlp('bash', 'bash', '-c', r'''
set -euo pipefail
git ls-files --cached --others --exclude-standard \
  | LC_ALL=C sort \
  | while IFS= read -r path; do
      case "$path" in
        benchmarks/evidence/*|docs/oryn2/IMPLEMENTATION_STATUS.md|docs/oryn2/CAPABILITY_MILESTONES.md) continue ;;
      esac
      [[ -f "$path" ]] || continue
      printf '%s\0' "$path"
      shasum -a 256 "$path" | awk '{print $1}'
    done \
  | shasum -a 256 \
  | awk '{print $1}'
''')

result = hashlib.sha256()
for path in paths:
    if not included(path):
        continue
    content = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            content.update(block)
    result.update(path)
    result.update(b'\0')
    # shasum prefixes its output with '\\' when the filename needs escaping;
    # awk retains that prefix in its first field. Usually Git already quotes
    # these paths, but preserve this behavior for any literal matching name.
    if b'\\' in path or b'\n' in path:
        result.update(b'\\')
    result.update(content.hexdigest().encode('ascii'))
    result.update(b'\n')
sys.stdout.write(result.hexdigest() + '\n')
PY
