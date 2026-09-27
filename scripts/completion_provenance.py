"""Exact artifact and evidence joins for the combined completion candidate.

This layer establishes provenance, not the truth of a result. A successful join
never upgrades a component result into security or combined-plan qualification.
"""
import hashlib
import copy
from contextlib import contextmanager, nullcontext
import importlib.util
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REQUIRED_ARTIFACTS = frozenset({
    'runtime', 'driver', 'workflow_executor', 'worker', 'source_snapshot',
    'v8_archive', 'bindings_source', 'icu_data', 'startup_snapshot',
    'engine_profile', 'engine_worker', 'external_references', 'worker_manifest', 'signature_evidence', 'fixture_contract', 'fixture_repairs', 'manual_procedures', 'workflow_contract',
    'framework_build', 'page_benchmark', 'group_benchmark', 'churn',
    'native_controls_contract', 'reference_browser', 'reference_driver', 'native_media_tests',
})
COMPONENTS = frozenset({'core', 'workflows', 'frameworks', 'q', 'security',
                        'architecture', 'reliability', 'native_prerequisites', 'fixture_topology', 'native_media'})


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def is_digest(value):
    return isinstance(value, str) and len(value) == 64 and all(c in '0123456789abcdef' for c in value)


def read_record(record):
    if not isinstance(record, dict) or not is_digest(record.get('sha256')):
        raise ValueError('artifact requires a SHA-256 identity')
    name = record.get('path')
    if not isinstance(name, str) or not name:
        raise ValueError('artifact requires an absolute path string')
    path = Path(name)
    if not path.is_absolute() or not path.is_file() or digest(path) != record['sha256']:
        raise ValueError('artifact missing or changed: ' + str(path))
    return path.resolve()


def render_contracts(root=ROOT):
    """Separate display-list and additive snapshot identities; never infer one from another."""
    declarations = {
        'render_contract_version': ('crates/oryn-common/src/v4.rs', 'RENDER_CONTRACT_VERSION'),
        'local_effects_contract_version': ('crates/oryn-common/src/v4.rs', 'LOCAL_EFFECTS_CONTRACT_VERSION'),
    }
    result = {}
    for field, (source, symbol) in declarations.items():
        matches = re.findall(r'^pub const ' + symbol + r': u16 = ([0-9]+);',
                             (Path(root) / source).read_text(), re.MULTILINE)
        if len(matches) != 1 or int(matches[0]) <= 0:
            raise ValueError('ambiguous rendering contract: ' + field)
        result[field] = int(matches[0])
    return result


def validate_render_contracts(manifest, expected):
    for field, version in expected.items():
        if type(manifest.get(field)) is not int or manifest[field] != version:
            raise ValueError('signed worker rendering contract mismatch: ' + field)


def validate_engine_source_snapshot(path, profile, source_index=None):
    # load_candidate already validates every archive entry against its index.
    # This joins the selected Rust crate (including otherwise ignored files)
    # into that index before the final immutable artifact digest recheck.
    import tarfile
    from completion_source_snapshot import INDEX
    if source_index is None:
        with tarfile.open(path, 'r:') as archive:
            index = json.load(archive.extractfile(INDEX))
    else:
        # Only the enclosing candidate transaction supplies its freshly
        # validated index; standalone callers keep the archive-read path.
        index = source_index
    records = {record['path']: record['sha256'] for record in index['files']}
    manifest_path = ROOT / 'vendor/rusty-v8/oryn-source-manifest.json'
    manifest = json.loads(manifest_path.read_text())
    expected = {'vendor/rusty-v8/' + name: value for name, value in manifest['files'].items()}
    expected.update({'vendor/rusty-v8/oryn-source-manifest.json': profile['rusty_v8_source_manifest_sha256'],
                     'patches/rusty-v8/weak-callback-teardown.patch': profile['rusty_v8_weak_teardown_patch_sha256'],
                     'patches/rusty-v8/weak-callback-teardown.json': profile['rusty_v8_patch_metadata_sha256'],
                     'Cargo.toml': digest(ROOT / 'Cargo.toml'), 'Cargo.lock': digest(ROOT / 'Cargo.lock')})
    for name, value in expected.items():
        if records.get(name) != value:
            raise ValueError('candidate source snapshot omits or changes selected Rust V8 input: ' + name)


def validate_engine(records, artifacts, source_tree_sha256, source_index=None):
    """Re-use the current engine-profile validator; no shell command or signing occurs."""
    spec = importlib.util.spec_from_file_location('completion_engine_profile', ROOT / 'scripts/validate-v8-runtime-profile.py')
    validator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(validator)
    profile = validator.validate(json.loads(artifacts['engine_profile'].read_text()), ROOT, profile='core-full')
    for engine_name, name in dict(archive='v8_archive', binding='bindings_source',
                                 icu_data='icu_data', startup_snapshot='startup_snapshot',
                                 external_references='external_references', worker='engine_worker').items():
        if (Path(profile[engine_name + '_path']).resolve() != artifacts[name]
                or profile[engine_name + '_sha256'] != records[name]['sha256']):
            raise ValueError('combined candidate differs from engine profile: ' + name)
    validate_engine_source_snapshot(artifacts['source_snapshot'], profile, source_index)
    # Signing modifies the executable. The profile's unsigned worker and the
    # signed deployed worker have separate identities; do not conflate them.
    manifest = json.loads(artifacts['worker_manifest'].read_text())
    validate_render_contracts(manifest, render_contracts(ROOT))
    for field, expected in dict(source_tree_sha256=source_tree_sha256,
                                worker_build_sha256=records['engine_worker']['sha256'],
                                v8_archive_sha256=records['v8_archive']['sha256'],
                                v8_profile_sha256=records['engine_profile']['sha256'],
                                v8_binding_sha256=records['bindings_source']['sha256'],
                                **{field: profile[field] for field in ('rusty_v8_source_manifest_sha256', 'rusty_v8_source_tree_sha256',
                                    'rusty_v8_weak_teardown_patch_sha256', 'rusty_v8_patch_metadata_sha256')}).items():
        if manifest.get(field) != expected:
            raise ValueError('signed worker manifest differs from candidate: ' + field)
    return profile


def validate_source_snapshot(path, source_tree_sha256):
    from completion_source_snapshot import validate
    return validate(path, source_tree_sha256)


@contextmanager
def candidate_transaction(path, *, phase=None):
    """One invocation's canonical checks around additional candidate consumers.

    No validated object is cached. The yielded object is valid only inside this
    boundary; successful exit rechecks the private original inventory. A caller
    error stays primary if the final integrity boundary also fails.
    """
    measure = phase if phase is not None else lambda name: nullcontext()
    selected_path = Path(path).absolute()
    path = selected_path.resolve()
    candidate_bytes = path.read_bytes()
    candidate_sha256 = hashlib.sha256(candidate_bytes).hexdigest()
    candidate = json.loads(candidate_bytes)
    if not isinstance(candidate, dict) or type(candidate.get('schema_version')) is not int or candidate.get('schema_version') != 1 or not is_digest(candidate.get('source_tree_sha256')):
        raise ValueError('candidate needs version and complete source-tree identity')
    records = candidate.get('artifacts', {})
    if not isinstance(records, dict) or not REQUIRED_ARTIFACTS <= records.keys():
        raise ValueError('combined candidate artifact identities incomplete')
    with measure('inventory.initial'):
        artifacts = {name: read_record(record) for name, record in records.items()}
    # Keep all validation baselines private: callback mutation is not authority.
    bound = dict(path=path, sha256=candidate_sha256, record=copy.deepcopy(candidate),
                 artifacts=dict(artifacts))
    def final_check():
        with measure('inventory.final'):
            if 'security_prebuilt' in artifacts:
                # Added resources are invisible to a sweep of known file hashes.
                from security_prebuilt import validate_bundle
                validate_bundle(json.loads(artifacts['security_prebuilt'].read_text()),
                                dict(record=candidate, artifacts=artifacts))
            for name, record in records.items():
                if read_record(record) != artifacts[name]:
                    raise ValueError('candidate artifact destination changed during validation: ' + name)
            if selected_path.resolve() != path:
                raise ValueError('candidate manifest destination changed during validation')
            if digest(path) != candidate_sha256:
                raise ValueError('candidate manifest changed during validation')
            if bound != dict(path=path, sha256=candidate_sha256, record=candidate, artifacts=artifacts):
                raise ValueError('validated candidate binding changed during transaction')
    try:
        with measure('candidate.semantics'):
            with measure('candidate.source_snapshot'):
                source_index = validate_source_snapshot(
                    artifacts['source_snapshot'], candidate['source_tree_sha256'])
            with measure('candidate.engine'):
                validate_engine(records, artifacts, candidate['source_tree_sha256'], source_index)
            # The contract validated by OIL must be the same bytes captured in the shared
            # candidate. It cannot be substituted by an unrelated file with a familiar name.
            with measure('candidate.workflow'):
                contract = ROOT / 'test-harness/workflows/five-family-contract-v1.json'
                if digest(contract) != records['workflow_contract']['sha256']:
                    from workflow_input_lock import load as load_workflow_lock
                    load_workflow_lock(artifacts['workflow_contract'], candidate['source_tree_sha256'], ROOT)
            if 'core_profile_registry' in artifacts:
                with measure('candidate.registry'):
                    from core_profile_contract import validate_registry
                    validate_registry(dict(path=path, sha256=candidate_sha256, record=candidate, artifacts=artifacts), required=True)
            if 'architecture_prebuilt' in artifacts:
                with measure('candidate.architecture'):
                    from architecture_prebuilt import validate_preparation
                    validate_preparation({'path': path, 'sha256': candidate_sha256, 'record': candidate, 'artifacts': artifacts})
            if 'security_prebuilt' in artifacts:
                with measure('candidate.security'):
                    # Known-file digests alone do not detect an added bundle resource.
                    # Recheck exact membership at every candidate integrity boundary.
                    from security_prebuilt import validate_bundle
                    validate_bundle(json.loads(artifacts['security_prebuilt'].read_text()),
                                    dict(record=candidate, artifacts=artifacts))
        yield bound
    except BaseException as failure:
        try:
            final_check()
        except BaseException as integrity_failure:
            failure.add_note('candidate final integrity check failed: ' + str(integrity_failure))
        raise
    else:
        final_check()


def load_candidate(path):
    """Standalone callers retain the complete initial/final validation boundary."""
    with candidate_transaction(path) as bound:
        result = bound
    return result


def workflow_identity(bound, binary, worker, source_tree_sha256):
    records = bound['record']['artifacts']
    for value, name in ((binary, 'workflow_executor'), (worker, 'worker')):
        if Path(value).resolve() != bound['artifacts'][name] or digest(value) != records[name]['sha256']:
            raise ValueError('workflow executable differs from combined candidate: ' + name)
    if source_tree_sha256 != bound['record']['source_tree_sha256']:
        raise ValueError('working source differs from combined candidate')
    return dict(parent_sha256=records['workflow_executor']['sha256'],
                worker_sha256=records['worker']['sha256'],
                source_tree_sha256=source_tree_sha256, candidate_manifest_sha256=bound['sha256'])


def join(candidate_path, components):
    """Verify producer, raw inputs and report bytes for all required domains.

    The component manifest is an evidence index, never a collection of manually
    asserted pass flags. Domain validation remains separately visible, including
    failures. Raw inputs and producer source hashes are preserved on output.
    """
    candidate = load_candidate(candidate_path)
    if not isinstance(components, dict) or set(components) != COMPONENTS:
        raise ValueError('exact combined component set required')
    output = {}
    for name, item in sorted(components.items()):
        if not isinstance(item, dict):
            raise ValueError('component evidence must be an object: ' + name)
        if item.get('candidate_manifest_sha256') != candidate['sha256']:
            raise ValueError('component candidate mismatch: ' + name)
        producer = item.get('producer_sources')
        inputs = item.get('inputs')
        if not isinstance(producer, list) or not producer or not isinstance(inputs, list) or not inputs:
            raise ValueError('component requires producer sources and raw inputs: ' + name)
        for group in (producer, inputs):
            paths = [read_record(record) for record in group]
            if len(paths) != len(set(paths)):
                raise ValueError('duplicate component input: ' + name)
        report_path = read_record(item.get('report'))
        report = json.loads(report_path.read_text())
        # Binding must occur in the report itself. An index cannot relabel older
        # evidence as the current candidate after its execution.
        if not isinstance(report, dict):
            raise ValueError('component report must be an object: ' + name)
        binding = report.get('candidate_manifest_sha256')
        if name == 'core':
            binding = report.get('candidate_sha256')
        if binding != candidate['sha256']:
            raise ValueError('raw report candidate mismatch: ' + name)
        if name == 'core' and 'core_profile_registry' in candidate['artifacts']:
            from core_profile_contract import validate_core_aggregate
            validate_core_aggregate(report, candidate)
        output[name] = dict(report=item['report'], producer_sources=producer, inputs=inputs)
    return dict(schema_version=1, candidate_manifest_sha256=candidate['sha256'],
                source_tree_sha256=candidate['record']['source_tree_sha256'],
                artifacts=candidate['record']['artifacts'], components=output,
                provenance_complete=True, qualified=False,
                scope='Exact provenance join only; component validators and combined acceptance remain mandatory')
