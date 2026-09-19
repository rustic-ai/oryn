"""Assemble repair44 from immutable repair43. Default preview is read-only.

Root alone invokes write after independent review and a separate exact-hash approval.
No generator, formatter, build, gate, test, or environment helper is executed here.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / 'artifacts/completion-validation-2026-09-18'
BASELINE = ROOT / 'docs/oryn2/GENERATION-COMPLETE-REPAIR-43-2026-09-18.json'
BASELINE_SHA = 'c29cea542382ebe3f74b785c533f8b8db069142b3f7d20332de1050b6da0549c'
OLD_LEASE = BASE / 'repair43-whole-source-lease-inputs-01.json'
OLD_LEASE_SHA = '4e8ba03f573bec8c09aefdf7eb699f1f95b7df1adcecadc7b63f1d2bc7fd3f0d'
GATE44 = ROOT / 'docs/oryn2/GENERATION-COMPLETE-REPAIR-44-2026-09-18.json'
PYTHON11 = dict(path='artifacts/completion-validation-2026-09-18/fq-python-regressions-11/preflight.json', sha256='03b527a9c69324c68966cb24909bb6dba0e116bd805a448a332d70d4b0079cb3')
PYTHON07 = dict(path='artifacts/completion-validation-2026-09-18/fq-python-regressions-07/preflight.json', sha256='b771fe6c9c532bc6cfaecdc00b44612c960386c01d30f38c68da3e850a09d687')
SOURCE_SCOPES = {'sources','reviewed_sources','source_inventory','formatting_sources','accepted_sources'}
ROLES = ('implementation_sources', 'test_sources')
PARENTS = {f'{group}-{number:02}' for group, count in [('G',8),('R',7),('P',6),('L',5),('W',5),('S',5),('V',5),('F',4),('Q',6)] for number in range(1,count+1)}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encode(data):
    return (json.dumps(data, indent=2, ensure_ascii=False) + '\n').encode()


def repo_path(value):
    path = Path(value)
    if path.is_absolute() or '..' in path.parts:
        raise ValueError('not a repository-relative path: ' + value)
    full = ROOT / path
    if not full.resolve().is_relative_to(ROOT.resolve()):
        raise ValueError('path resolves outside repository: ' + value)
    return full


def rel(path):
    return str(path.relative_to(ROOT))


def read_record(record):
    path = repo_path(record['path'])
    data = path.read_bytes()
    if digest(data) != record['sha256']:
        raise ValueError('input digest mismatch: ' + record['path'])
    return data


def pointer(document, value):
    if not value:
        return document
    if not value.startswith('/'):
        raise ValueError('JSON evidence pointer must start with /')
    for part in value[1:].split('/'):
        key = part.replace('~1', '/').replace('~0', '~')
        document = document[int(key)] if isinstance(document, list) else document[key]
    return document


def included(path):
    p = Path(path)
    return (p.parts[0] in ('crates','vendor','scripts','tools','docs')
            and not any(x in ('target','node_modules','__pycache__','.git') for x in p.parts[:-1])
            and not p.name.endswith(('.pyc','.pyo','.orig','.rej','.bak'))
            and not p.name.startswith('GENERATION-COMPLETE'))


def discover(roots):
    result = set()
    for top in roots:
        for directory, dirs, files in os.walk(ROOT / top):
            dirs[:] = [x for x in dirs if x not in ('target','node_modules','__pycache__','.git')]
            for name in files:
                path = rel(Path(directory) / name)
                if included(path):
                    result.add(path)
    return result


def assemble(config, config_record):
    errors = []
    def problem(code, **context):
        errors.append(dict(code=code, **context))
    baseline = json.loads(read_record(dict(path=rel(BASELINE), sha256=BASELINE_SHA)))
    old_lease = json.loads(read_record(dict(path=rel(OLD_LEASE), sha256=OLD_LEASE_SHA)))
    if digest((ROOT/'docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md').read_bytes()) != baseline['plan_sha256']:
        raise ValueError('full completion plan changed from immutable43')
    if set(baseline['requirements']) != PARENTS:
        raise ValueError('baseline does not contain exactly51 parents')
    baseline_bound = {row['path'] for item in baseline['requirements'].values() for role in ROLES for row in item[role]}
    if len(baseline_bound) != 909 or len(baseline['review_sources']) != 1006 or len(old_lease['sources']) != 19450:
        raise ValueError('immutable43 denominator mismatch')
    old_sources = {row['path']: row for row in old_lease['sources']}
    if len(old_sources) != 19450 or old_lease['source_count'] != 19450:
        raise ValueError('duplicate/inconsistent historical whole-source paths')
    if old_lease['authored_discovery_roots'] != ['crates','vendor','scripts','tools','docs']:
        raise ValueError('unexpected historical discovery roots')
    declaration = copy.deepcopy(baseline)
    declaration.update(phase='post_generation_build_and_regression_repair_44', qualification_complete=False,
                       previous_declaration=dict(path=rel(BASELINE), sha256=BASELINE_SHA),
                       parent_declaration=dict(path=rel(BASELINE), sha256=BASELINE_SHA))
    declaration['review_notes'].extend(config.get('review_notes', []))
    # Historical reviews retain exact order, objects, paths and hashes.
    reviews = copy.deepcopy(baseline['review_sources'])
    review_by_path = {row['path']: row['sha256'] for row in reviews}
    if len(review_by_path) != 1006:
        raise ValueError('historical review path collision')
    def add_review(row):
        path, sha = row['path'], row['sha256']
        try:
            read_record(row)
            if path in review_by_path and review_by_path[path] != sha:
                raise ValueError('historical/current review hash collision')
            if path not in review_by_path:
                reviews.append(dict(path=path, sha256=sha))
                review_by_path[path] = sha
        except (OSError, ValueError) as error:
            problem('review_custody', path=path, error=str(error))
    for row in list(reviews):
        add_review(row)
    for row in [dict(path=rel(BASELINE),sha256=BASELINE_SHA), dict(path=rel(OLD_LEASE),sha256=OLD_LEASE_SHA), config_record,PYTHON11,PYTHON07] + config.get('review_sources', []):
        add_review(row)
    def canonical_identity(value):
        known = {'candidate_reconciliation','critical_source_review','failure_coverage','fixture_readiness','qualification_execution','release_prerequisites','service_startup_completion','service_startup_review'}
        if value in known:
            return '/root/' + value
        if value == '/root' or (isinstance(value,str) and value.startswith('/root/') and value[6:] in known):
            return value
        raise ValueError('missing or unknown team author/reviewer identity')

    def accepted_review(reference, expected_reviewer, author=None, require_author=False):
        add_review(reference)
        document = json.loads(read_record(reference))
        reviewer = canonical_identity(document.get('reviewer'))
        if reviewer != canonical_identity(expected_reviewer):
            raise ValueError('actual reviewer identity is not the expected independent reviewer')
        if require_author and reviewer == canonical_identity(author):
            raise ValueError('reviewer is the source/map/binding author')
        if not isinstance(document.get('status'),str) or not document['status'].startswith('accepted'):
            raise ValueError('review status is not accepted')
        if document.get('findings') != [] or document.get('open_findings',[]) != []:
            raise ValueError('review does not explicitly close all findings')
        return document
    def evidence(claim, proofs, purpose='source'):
        if not proofs:
            problem('missing_independent_evidence',path=claim['path'],purpose=purpose)
            return
        for proof in proofs:
            reference = proof['reference']
            try:
                document = accepted_review(reference,proof.get('reviewer'),claim.get('author'),require_author=True)
                scope = proof['scope']
                allowed = SOURCE_SCOPES if purpose == 'source' else SOURCE_SCOPES - {'formatting_sources'} if purpose == 'semantic_source' else {'accepted_binding_maps'} if purpose == 'map' else {'accepted_bindings'}
                if scope not in allowed:
                    raise ValueError('reference/input arrays cannot substitute for accepted scope')
                index = proof['index']
                if not isinstance(index,int) or isinstance(index,bool) or index < 0:
                    raise ValueError('accepted scope requires an explicit nonnegative record index')
                item = document[scope][index]
                if item['path'] != claim['path'] or item['sha256'] != claim['sha256']:
                    raise ValueError('accepted scope does not pin this exact source/map hash')
                if purpose == 'binding' and any(item[key] != claim[key] for key in ('requirement','role')):
                    raise ValueError('accepted binding does not name this exact parent/role')
                if purpose == 'source' and scope == 'formatting_sources' and claim.get('kind') != 'format_only':
                    semantic_sha = claim.get('semantic_sha256')
                    if not semantic_sha or item.get('before_sha256') != semantic_sha:
                        raise ValueError('final formatting must preserve an exact accepted semantic before hash')
                    evidence(dict(claim,sha256=semantic_sha),claim.get('semantic_review_evidence',[]),purpose='semantic_source')
                if claim.get('kind') == 'format_only':
                    if scope != 'formatting_sources' or item.get('before_sha256') != claim.get('before_sha256'):
                        raise ValueError('format-only requires exact43 before hash in accepted formatting scope')
            except (OSError,ValueError,KeyError,TypeError,IndexError,AttributeError) as error:
                problem('invalid_independent_evidence',path=claim['path'],reference=reference['path'],error=str(error))
    hints, approved_map_paths = {}, set()
    maps = []
    baseline_roles = {(parent,role,row['path']): row['sha256'] for parent,item in baseline['requirements'].items() for role in ROLES for row in item[role]}
    def add_binding(parent, role, path, sha):
        if parent not in PARENTS or role not in ROLES:
            raise ValueError('unknown parent or source role')
        repo_path(path)
        rows = declaration['requirements'][parent][role]
        existing = next((row for row in rows if row['path'] == path), None)
        if existing is None:
            rows.append(dict(path=path,sha256=sha))
        # Hash updates are resolved once per path from the separately approved final ledger.
    for entry in config.get('binding_maps', []):
        reference = entry['reference']
        data = json.loads(read_record(reference))
        maps.append(reference)
        add_review(reference)
        evidence(dict(reference,author=entry.get('author')),entry.get('review_evidence',[]),purpose='map')
        if data.get('baseline_declaration', {}).get('sha256') not in (None, BASELINE_SHA):
            problem('map_baseline_mismatch', path=reference['path'])
        for row in data.get('hash_updates', []):
            key = (row['requirement'],row['role'],row['path'])
            if key not in baseline_roles or row['before_sha256'] != baseline_roles[key]:
                problem('map_update_drops_or_rebinds_prior_role', path=row['path'], requirement=row['requirement'], role=row['role'])
            hints[row['path']] = row['sha256']
            approved_map_paths.add(row['path'])
        for row in data.get('binding_additions', []):
            add_binding(row['requirement'],row['role'],row['path'],row['sha256'])
            hints[row['path']] = row['sha256']
            approved_map_paths.add(row['path'])
        for row in data.get('sources', []):
            if 'path' in row and 'sha256' in row:
                hints[row['path']] = row['sha256']
                approved_map_paths.add(row['path'])
        for row in data.get('root_status_document_changes', []):
            hints[row['path']] = row['sha256']
            approved_map_paths.add(row['path'])
        # A supplement may carry exact later source hashes without replacing its older map.
        for row in data.get('final_hash_overrides', []):
            hints[row['path']] = row['sha256']
            approved_map_paths.add(row['path'])
    for row in config.get('binding_additions', []):
        add_binding(row['requirement'],row['role'],row['path'],row['sha256'])
        evidence(row,row.get('review_evidence',[]),purpose='binding')
        hints[row['path']] = row['sha256']
    authorizations = {}
    for row in config.get('final_sources', []):
        if row['path'] in authorizations:
            raise ValueError('duplicate final source authorization: ' + row['path'])
        repo_path(row['path'])
        if row['kind'] not in ('code','fixture','historical_dependency','coordinator_doc','format_only'):
            raise ValueError('unrecognized final source authorization kind')
        if not row.get('author'):
            problem('missing_source_author_identity',path=row['path'])
        if row['path'] == rel(Path(__file__).resolve()) and row.get('author') != '/root/candidate_reconciliation':
            problem('assembler_author_identity_mismatch',path=row['path'])
        if row['kind'] == 'coordinator_doc' and (row['path'] not in old_sources or not row['path'].startswith('docs/') or Path(row['path']).suffix != '.md'):
            problem('coordinator_exemption_is_not_existing_markdown',path=row['path'])
        if row['kind'] == 'format_only' and row['path'] not in old_sources:
            problem('new_source_cannot_use_format_only_exemption',path=row['path'])
        authorizations[row['path']] = row
        evidence(row, row.get('review_evidence', []))
        if row['path'] in old_sources and row.get('before_sha256') != old_sources[row['path']]['sha256']:
            problem('final_source_baseline_mismatch', path=row['path'])
    bound = {row['path'] for item in declaration['requirements'].values() for role in ROLES for row in item[role]}
    for path,row in authorizations.items():
        if row['kind'] not in ('coordinator_doc','format_only') and path not in bound:
            problem('approved_code_has_no_parent_role', path=path)
    assembler_path = rel(Path(__file__).resolve())
    assembler_roles = {(parent,role,row['path']) for parent,item in declaration['requirements'].items() for role in ROLES for row in item[role]}
    if assembler_path not in authorizations:
        problem('assembler_final_source_authorization_missing',path=assembler_path)
    if ('Q-01','implementation_sources',assembler_path) not in assembler_roles:
        problem('assembler_Q01_implementation_binding_missing',path=assembler_path)
    found = discover(old_lease['authored_discovery_roots'])
    universe = set(old_sources) | found | bound | set(authorizations)
    actual, missing, changes, unknown = {}, [], [], []
    for path in sorted(universe):
        try:
            full = repo_path(path)
            before = full.stat()
            data = full.read_bytes()
            after = full.stat()
            if (before.st_size,before.st_mtime_ns,before.st_ino) != (after.st_size,after.st_mtime_ns,after.st_ino):
                raise ValueError('source changed while hashing')
            record = dict(path=path,sha256=digest(data),bytes=len(data))
            actual[path] = record
        except (OSError, ValueError) as error:
            missing.append(path)
            problem('missing_or_unstable_source',path=path,error=str(error))
            continue
        prior = old_sources.get(path)
        changed = prior is None or prior['sha256'] != record['sha256']
        if changed:
            changes.append(dict(path=path,before_sha256=prior['sha256'] if prior else None,sha256=record['sha256']))
            if path not in authorizations:
                unknown.append(path)
                problem('source_delta_requires_final_review',path=path,observed_sha256=record['sha256'],map_hint_sha256=hints.get(path))
        if path in authorizations and authorizations[path]['sha256'] != record['sha256']:
            problem('approved_final_hash_changed',path=path,expected=authorizations[path]['sha256'],actual=record['sha256'])
    # Every occurrence of an old path retains its parent/role; only an approved final hash can change it.
    for parent,item in declaration['requirements'].items():
        for role in ROLES:
            for row in item[role]:
                path = row['path']
                if path in authorizations:
                    row['sha256'] = authorizations[path]['sha256']
                elif path in actual and row['sha256'] != actual[path]['sha256']:
                    problem('bound_hash_requires_final_review',path=path,requirement=parent,role=role)
    current_roles = {(parent,role,row['path']) for parent,item in declaration['requirements'].items() for role in ROLES for row in item[role]}
    if not set(baseline_roles).issubset(current_roles) or not baseline_bound.issubset(bound):
        raise ValueError('assembly dropped a historical source role')
    for parent,item in baseline['requirements'].items():
        for key,value in item.items():
            if key not in ROLES and declaration['requirements'][parent][key] != value:
                raise ValueError('assembly changed historical requirement metadata')
    # Full Python union; no collection, imports or test execution.
    old_python = json.loads(read_record(PYTHON11))['test_sources']
    oldest_python = json.loads(read_record(PYTHON07))['test_sources']
    if len(oldest_python) != 45 or len(set(oldest_python)) != 45 or len(old_python) != 102:
        raise ValueError('historical Python source denominator mismatch')
    python_paths = set(old_python) | set(oldest_python) | {path for path in bound if Path(path).suffix == '.py' and Path(path).name.startswith('test')}
    for directory in (ROOT/'scripts',ROOT/'tools/wptrunner-oryn/tests'):
        python_paths.update(rel(path) for path in directory.rglob('test*.py'))
    for path in sorted(python_paths - bound):
        problem('python_regression_not_bound',path=path)
    python_sources = [actual[path] for path in sorted(python_paths) if path in actual]
    inventory_ref = config.get('evidence_inventory')
    if inventory_ref is None:
        problem('complete_evidence_inventory_review_missing')
    else:
        try:
            inventory = accepted_review(inventory_ref,config.get('evidence_inventory_reviewer'))
            if inventory.get('scope') != 'repair44_complete_evidence_inventory':
                raise ValueError('inventory review does not cover complete repair44 evidence')
            if inventory.get('historical_boundaries') != [dict(path=rel(BASELINE),sha256=BASELINE_SHA),dict(path=rel(OLD_LEASE),sha256=OLD_LEASE_SHA)]:
                raise ValueError('evidence inventory changes historical boundaries')
            entries = {row['path']:row['sha256'] for row in inventory['inputs']}
            if len(entries) != len(inventory['inputs']):
                raise ValueError('duplicate evidence inventory paths')
            for row in inventory['inputs']:
                add_review(row)
            exempt = {row['path'] for row in baseline['review_sources']} | {rel(BASELINE),rel(OLD_LEASE),config_record['path'],inventory_ref['path']}
            for row in reviews:
                if row['path'] not in exempt and entries.get(row['path']) != row['sha256']:
                    problem('review_graph_input_not_in_accepted_inventory',path=row['path'])
        except (OSError,ValueError,KeyError,TypeError,AttributeError) as error:
            problem('invalid_complete_evidence_inventory',error=str(error))
    closure_ref = config.get('source_scope_closure')
    if closure_ref is None:
        problem('source_scope_closure_review_missing',open_scope='Native residual allocations and service startup142 remain explicit; no generic ready flag closes them.')
    else:
        try:
            closure = accepted_review(closure_ref,config.get('source_scope_closure_reviewer'))
            if closure.get('scope') != 'repair44_full_source_batch' or closure.get('source_generation_complete') is not True or closure.get('qualification_complete') is not False:
                raise ValueError('closure does not distinguish complete source from unqualified execution')
            if closure.get('binding_maps') != maps or closure.get('final_sources_sha256') != digest(encode(config.get('final_sources',[]))) or closure.get('evidence_inventory') != inventory_ref:
                raise ValueError('closure does not bind this exact map/source/evidence population')
            if not isinstance(closure.get('unresolved_obligations'),list):
                raise ValueError('closure must retain an explicit unresolved-obligation ledger')
        except (OSError,ValueError,KeyError,TypeError,AttributeError) as error:
            problem('invalid_source_scope_closure',error=str(error))
    # Preserve the earlier lease's evidence inventory; new review references are additive.
    lease = copy.deepcopy(old_lease)
    lease.update(kind='repair44_whole_source_and_review_lease_inputs',
                 baseline=dict(path=rel(OLD_LEASE),sha256=OLD_LEASE_SHA),
                 expected_declaration_path=rel(GATE44), source_count=len(universe),
                 sources=[actual.get(path,old_sources.get(path,dict(path=path,sha256=None))) for path in sorted(universe)],
                 review_sources=copy.deepcopy(reviews),
                 declaration_digest_binding='Actual repair44 declaration digest is recorded externally after writing; it is intentionally absent from this lease to avoid a cycle.',
                 repair44_assembly_inputs=dict(config=config_record,binding_maps=maps),
                 repair44_changes=changes, execution_performed=False)
    # Historical fields remain historical. These explicit current steps supersede their prose only.
    lease['repair44_admission_steps'] = ['Root reviews the final payload and exact write approval.',
        'Write this lease without an actual44 declaration digest.',
        'Add this lease digest to the new declaration review sources and write the declaration last.',
        'Record both actual digests externally; invoke only reviewed native30/Python12 custody dispatch.']
    lease_path = repo_path(config['lease_output_path'])
    if not lease_path.resolve().is_relative_to(BASE.resolve()) or lease_path.name.startswith('GENERATION-COMPLETE'):
        raise ValueError('new whole lease must be a completion artifact')
    if rel(lease_path) in review_by_path or rel(lease_path) in universe or rel(GATE44) in universe or rel(GATE44) in review_by_path:
        raise ValueError('new lease/declaration would form a source/review hash cycle')
    lease_bytes = encode(lease)
    declaration['review_sources'] = reviews + [dict(path=rel(lease_path),sha256=digest(lease_bytes))]
    declaration_bytes = encode(declaration)
    if config.get('ready_for_root_write') is not True:
        problem('root_final_source_review_not_closed')
    # Recheck every observed input after assembly, without changing proposed identities.
    for record in list(actual.values()) + reviews:
        try:
            read_record(record)
        except (OSError,ValueError) as error:
            problem('input_changed_during_assembly',path=record['path'],error=str(error))
    if discover(old_lease['authored_discovery_roots']) != found:
        problem('authored_source_census_changed_during_assembly')
    report = dict(status='ready_for_root_review' if not errors else 'draft_blocked_not_approval',
        baseline_declaration_sha256=BASELINE_SHA, baseline_whole_lease_sha256=OLD_LEASE_SHA,
        assembler=dict(path=rel(Path(__file__).resolve()),sha256=digest(Path(__file__).read_bytes())),
        configuration=config_record, parent_count=len(PARENTS), prior_bound_paths=len(baseline_bound),
        bound_paths=len(bound), prior_review_references=1006, review_references=len(declaration['review_sources']),
        prior_whole_source_paths=19450, whole_source_paths=len(universe), python_source_count=len(python_paths),
        preserved_prior_parent_roles=len(baseline_roles), changes=changes, missing_paths=missing,
        source_paths_awaiting_final_review=unknown, blockers=errors, python_sources=python_sources,
        proposed_lease=dict(path=rel(lease_path),sha256=digest(lease_bytes)),
        proposed_declaration=dict(path=rel(GATE44),sha256=digest(declaration_bytes)),
        qualification_complete=False, execution_performed=False)
    return report, declaration_bytes, lease_bytes


def exclusive(path,data):
    with path.open('xb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode',nargs='?',default='preview',choices=('preview','write'))
    parser.add_argument('--config',type=Path,required=True)
    parser.add_argument('--config-sha256',required=True)
    parser.add_argument('--approval',type=Path)
    parser.add_argument('--approval-sha256')
    args = parser.parse_args()
    config_path = args.config.resolve()
    config_record = dict(path=rel(config_path),sha256=args.config_sha256)
    config = json.loads(read_record(config_record))
    report, declaration_bytes, lease_bytes = assemble(config, config_record)
    if args.mode == 'preview':
        print(encode(report).decode(),end='')
        return 0 if not report['blockers'] else 2
    if report['blockers']:
        raise ValueError('write refused: preview has unresolved blockers')
    if args.approval is None or not args.approval_sha256:
        parser.error('root write requires a separate exact-hash approval artifact')
    approval_ref = dict(path=rel(args.approval.resolve()),sha256=args.approval_sha256)
    approval = json.loads(read_record(approval_ref))
    expected = dict(action='root_authorized_repair44_write',status='approved',
        assembler_sha256=report['assembler']['sha256'],configuration_sha256=args.config_sha256,
        preview_sha256=digest(encode(report)),declaration_sha256=digest(declaration_bytes),
        whole_lease_sha256=digest(lease_bytes))
    if any(approval.get(key) != value for key,value in expected.items()):
        raise ValueError('root approval does not bind this exact reviewed preview/payload')
    lease_path = repo_path(report['proposed_lease']['path'])
    receipt_path = repo_path(config['write_receipt_path'])
    if not receipt_path.resolve().is_relative_to(BASE.resolve()):
        raise ValueError('write receipt must be a new completion artifact')
    if len({GATE44.resolve(),lease_path.resolve(),receipt_path.resolve(),config_path,args.approval.resolve()}) != 5:
        raise ValueError('outputs, configuration and approval must be distinct and non-aliasing')
    inputs = json.loads(lease_bytes)['sources'] + json.loads(lease_bytes)['review_sources']
    input_paths = {repo_path(row['path']).resolve() for row in inputs}
    if any(path.resolve() in input_paths for path in (GATE44,lease_path,receipt_path)):
        raise ValueError('output aliases a bound source or review input')
    if any(path.exists() for path in (GATE44,lease_path,receipt_path)):
        raise ValueError('write refused: preserve existing declaration/lease/receipt')
    # Reassemble once after approval; no source hash can be refreshed silently.
    reread, current_declaration, current_lease = assemble(config,config_record)
    if encode(reread) != encode(report) or current_declaration != declaration_bytes or current_lease != lease_bytes:
        raise ValueError('inputs changed after approved preview')
    receipt = dict(action='root_repair44_write',approval=approval_ref,approved_payload=expected,completed=False)
    try:
        exclusive(lease_path,lease_bytes)
        exclusive(GATE44,declaration_bytes)
        receipt['completed'] = True
    finally:
        # Partial outputs are immutable evidence; a failed write never deletes them.
        exclusive(receipt_path,encode(receipt))
    print(encode(receipt).decode(),end='')
    return 0


if __name__ == '__main__':
    sys.exit(main())
