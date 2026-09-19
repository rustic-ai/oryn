"""Fail-closed reviewed assertion dispositions; never rewrite raw WPT outcomes.

Policies are candidate inputs. Applications are independently reviewed statements
about one preserved raw assertion, not permission to add decisions after a run.
"""
import hashlib
import json
from pathlib import Path
from urllib.parse import urlsplit

from completion_provenance import is_digest, read_record

ROOT = Path(__file__).resolve().parents[1]
APPROVED_LEDGER = ROOT / 'benchmarks/conformance/upstream-test-defects-v1.json'
APPROVED_LEDGER_SHA256 = 'bd16881b11cd0beca373c1d0d95b0857a3d2d24167fd617c1b467741a8212370'
IDENTITY_FIELDS = ('candidate_sha256', 'registry_sha256', 'profile_id', 'profile_sha256',
                   'envelope_sha256', 'run_id', 'collection_id', 'attempt_id', 'original_run_id', 'platform')
SATISFIED_STATES = frozenset(('qualified_pass', 'satisfied_by_reviewed_disposition'))


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def object_digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'),
                                     ensure_ascii=False).encode('utf-8')).hexdigest()


def message_digest(message):
    if not isinstance(message, str):
        raise ValueError('assertion message must be an exact string')
    return hashlib.sha256(message.encode('utf-8')).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def keys(value, expected, label):
    require(isinstance(value, dict) and set(value) == set(expected), label + ' fields differ from schema')


def text(value, label):
    require(isinstance(value, str) and bool(value.strip()), label + ' must be nonempty')
    return value


def version(value, previous, label, track):
    require(type(value) is int and 1 <= value <= 10000, label + ' version invalid')
    first = None; seen = set(); identity = None
    while value > 1:
        require(previous is not None, label + ' revised version lacks previous record')
        path = track(previous)
        require(str(path) not in seen, label + ' history cycle')
        seen.add(str(path)); old = json.loads(path.read_text())
        require(isinstance(old, dict), label + ' history is not an object')
        field = 'revision' if label == 'policy' else 'version'
        name = 'policy_id' if label == 'policy' else 'id'
        require(type(old.get(field)) is int and old[field] == value - 1, label + ' version history discontinuity')
        if first is None:
            first = old; identity = old.get(name)
        require(old.get(name) == identity, label + ' history changes identity')
        value = old[field]; previous = old.get('previous_' + label)
    require(previous is None, label + ' initial version has previous history')
    return first


def global_variant(row):
    parsed = urlsplit(row['url'])
    variant = ('?' + parsed.query if parsed.query else '') + ('#' + parsed.fragment if parsed.fragment else '')
    served = parsed.path
    suffixes = (('.any.serviceworker.html', 'serviceworker'), ('.any.sharedworker.html', 'sharedworker'),
                ('.any.worker.html', 'dedicatedworker'), ('.any.serviceworker-module.html', 'serviceworker-module'),
                ('.any.sharedworker-module.html', 'sharedworker-module'), ('.any.worker-module.html', 'dedicatedworker-module'),
                ('.worker.html', 'dedicatedworker'))
    realm = next((realm for suffix, realm in suffixes if served.endswith(suffix)), None)
    if realm is None:
        if row['path'].endswith('.any.js'):
            require(served.endswith('.any.html'), 'unknown generated global cannot receive disposition')
        realm = 'window'
    require(not row.get('metadata', {}).get('jsshell'), 'jsshell cannot use browser assertion disposition')
    return realm, variant


def profile_identity(context, candidate=None):
    require(isinstance(context, dict), 'reviewed dispositions require verified profile context')
    result = {key: context.get(key) for key in IDENTITY_FIELDS}
    for key, value in result.items():
        text(value, key)
        if key.endswith('_sha256'):
            require(is_digest(value), 'profile identity digest invalid: ' + key)
    require(result['platform'] in {'macOS', 'Linux', 'Windows'}, 'unknown disposition platform')
    if candidate is not None:
        require(result['candidate_sha256'] == candidate, 'profile candidate mismatch')
    return result


class Policy:
    """Validated frozen rules plus per-run application accounting and input custody."""
    def __init__(self, path, *, candidate, coverage, checkout, profile_context):
        self.inputs = {}
        self.bindings = {}
        self.context = profile_identity(profile_context, candidate['sha256'])
        self.candidate = candidate
        self.checkout = Path(checkout).resolve()
        self.coverage = {}
        for row in coverage:
            require(row['id'] not in self.coverage, 'duplicate disposition coverage ID')
            self.coverage[row['id']] = row
        self.path = Path(path).resolve()
        binding = candidate['record'].get('artifacts', {}).get('core_disposition_policy')
        require(binding is not None, 'candidate does not freeze a disposition policy')
        require(self.track(binding) == self.path, 'disposition policy is not candidate-frozen path')
        self.sha256 = binding['sha256']
        if 'path' in candidate:
            self.track({'path': str(Path(candidate['path']).resolve()), 'sha256': candidate['sha256']})
        self.record = json.loads(self.path.read_text())
        keys(self.record, ('schema_version', 'policy_id', 'revision', 'previous_policy',
                          'upstream_revision', 'manifest_sha256', 'inventory_policy_sha256',
                          'coverage_sha256', 'approved_ledger', 'decisions'), 'disposition policy')
        require(type(self.record['schema_version']) is int and self.record['schema_version'] == 1, 'unknown disposition policy schema')
        text(self.record['policy_id'], 'policy ID')
        from wpt_full_qualification import REVISION, MANIFEST_SHA256, POLICY_SHA256
        for key, expected in [('upstream_revision', REVISION), ('manifest_sha256', MANIFEST_SHA256),
                              ('inventory_policy_sha256', POLICY_SHA256)]:
            require(self.record[key] == expected, 'disposition policy pin mismatch: ' + key)
        require(self.record['coverage_sha256'] == profile_context.get('coverage_sha256'), 'disposition coverage pin mismatch')
        previous = version(self.record['revision'], self.record['previous_policy'], 'policy', self.track)
        if previous:
            require(previous.get('policy_id') == self.record['policy_id'], 'policy history changes identity')
        self.validate_approved_ledger(self.record['approved_ledger'])
        require(isinstance(self.record['decisions'], list), 'decisions must be explicit list')
        self.decisions = {}
        self.targets = {}
        for decision in self.record['decisions']:
            self.validate_decision(decision)
        self.applications = {}
        self.consumed = set()
        self.applications_sha256 = None
        self.recheck()

    def track(self, record):
        keys(record, ('path', 'sha256'), 'artifact record')
        path = read_record(record)
        name = str(path)
        require(name not in self.inputs or self.inputs[name] == record['sha256'], 'conflicting evidence identity')
        original = str(Path(record['path']))
        bound = (record['sha256'], name)
        require(original not in self.bindings or self.bindings[original] == bound, 'artifact destination changed')
        self.bindings[original] = bound
        self.inputs[name] = record['sha256']
        self.inputs[original] = record['sha256']
        return path

    def source(self, relative, expected):
        require(isinstance(relative, str) and not Path(relative).is_absolute(), 'source must be checkout-relative')
        path = (self.checkout / relative).resolve()
        require(self.checkout in path.parents and path.is_file(), 'source escapes or is absent from checkout')
        self.track({'path': str(path), 'sha256': expected})
        return path.read_text(encoding='utf-8')

    def validate_approved_ledger(self, binding):
        require(binding.get('sha256') == APPROVED_LEDGER_SHA256 and digest(APPROVED_LEDGER) == APPROVED_LEDGER_SHA256,
                'existing approved ledger changed')
        ledger = json.loads(self.track(binding).read_text())
        require(ledger['schema_version'] == 1 and ledger['upstream']['revision'] == self.record['upstream_revision'], 'approved ledger pin mismatch')
        imported = set()
        for defect in ledger['defects']:
            content = self.source(defect['path'], defect['source_sha256'])
            for predicate in defect['source_predicates']:
                require(predicate in content, 'approved source predicate missing')
            text(defect['assertion_name_contains'], 'legacy named assertion')
            for variant in defect['variants']:
                identifier = 'testharness:/' + defect['path'] + variant
                require(identifier not in imported, 'duplicate approved imported assertion scope')
                imported.add(identifier)
                row = self.coverage.get(identifier)
                require(row is not None and row.get('tier') == 'necessary' and row['sha256'] == defect['source_sha256'],
                        'approved ledger must retain its exact zero-Core Necessary scope')
        require(len(imported) == 8, 'approved ledger applicability changed')
        self.imported_core_count = 0

    def review(self, record, kind, decision, payload_hash):
        review = json.loads(self.track(record).read_text())
        keys(review, ('schema_version', 'kind', 'decision_id', 'decision_version', 'decision_sha256',
                      'author', 'reviewer', 'approved', 'rationale', 'evidence_artifacts'), kind)
        require(type(review['schema_version']) is int and review['schema_version'] == 1 and review['kind'] == kind, 'review schema/kind mismatch')
        require(review['decision_id'] == decision['id'] and type(review['decision_version']) is int and review['decision_version'] == decision['version']
                and review['decision_sha256'] == payload_hash, 'independent review decision mismatch')
        require(review['approved'] is True, 'decision lacks independent approval')
        author = text(review['author'], 'review author'); reviewer = text(review['reviewer'], 'independent reviewer')
        require(author.strip().casefold() != reviewer.strip().casefold() and author == decision['author'], 'decision review is not independent')
        text(review['rationale'], 'review rationale')
        require(isinstance(review['evidence_artifacts'], list) and review['evidence_artifacts'], 'review has no retained resolution/spec evidence')
        seen = set()
        for item in review['evidence_artifacts']:
            path = self.track(item)
            require(path not in seen, 'duplicate resolution evidence')
            seen.add(path)
        return review

    def validate_decision(self, decision):
        keys(decision, ('id', 'version', 'previous_decision', 'kind', 'author', 'upstream_revision',
                        'manifest_sha256', 'inventory_policy_sha256', 'targets', 'source_files',
                        'source_predicates', 'assertion', 'expected_affected_assertions',
                        'upstream_resolution', 'inventory_decision'), 'decision')
        identifier = text(decision['id'], 'decision ID')
        require(identifier not in self.decisions, 'duplicate decision ID')
        require(decision['kind'] == 'pinned_upstream_assertion_defect', 'unsupported disposition kind')
        text(decision['author'], 'decision author')
        previous = version(decision['version'], decision['previous_decision'], 'decision', self.track)
        if previous:
            require(previous.get('id') == identifier, 'decision history changes identity')
        for key in ('upstream_revision', 'manifest_sha256', 'inventory_policy_sha256'):
            require(decision[key] == self.record[key], 'decision pin mismatch: ' + key)
        payload = {key: value for key, value in decision.items() if key not in {'upstream_resolution', 'inventory_decision'}}
        payload_hash = object_digest(payload)
        self.review(decision['upstream_resolution'], 'independently_reviewed_upstream_resolution', decision, payload_hash)
        self.review(decision['inventory_decision'], 'versioned_inventory_decision', decision, payload_hash)
        keys(decision['assertion'], ('name', 'raw_status', 'message_sha256'), 'assertion')
        name = text(decision['assertion']['name'], 'full assertion name')
        require(decision['assertion']['raw_status'] == 'FAIL' and is_digest(decision['assertion']['message_sha256']), 'disposition only permits exact named FAIL')
        require(type(decision['expected_affected_assertions']) is int and decision['expected_affected_assertions'] == 1, 'assertion multiplicity must be exactly one')
        sources = decision['source_files']
        require(isinstance(sources, dict) and sources, 'decision source/helper inventory missing')
        contents = {path: self.source(path, expected) for path, expected in sources.items()}
        predicates = decision['source_predicates']
        require(isinstance(predicates, list) and predicates, 'source predicates missing')
        for predicate in predicates:
            keys(predicate, ('path', 'text', 'line', 'occurrences'), 'source predicate')
            value = text(predicate['text'], 'source predicate text')
            require(predicate['path'] in contents and type(predicate['line']) is int and predicate['line'] > 0
                    and type(predicate['occurrences']) is int and 1 <= predicate['occurrences'] <= 100, 'source predicate scope invalid')
            content = contents[predicate['path']]
            lines = content.splitlines(keepends=True)
            require(predicate['line'] <= len(lines) and ''.join(lines[predicate['line'] - 1:]).startswith(value)
                    and content.count(value) == predicate['occurrences'], 'source predicate location/multiplicity mismatch')
        require(isinstance(decision['targets'], list) and decision['targets'], 'finite exact decision targets required')
        local = set()
        for target in decision['targets']:
            keys(target, ('id', 'path', 'url', 'global', 'variant', 'source_sha256', 'profiles',
                          'platforms', 'expected_subtests'), 'decision target')
            row = self.coverage.get(target['id'])
            require(row is not None and row.get('tier') == 'core' and row.get('test_type') == 'testharness', 'decision requires exact Core testharness ID')
            require(target['id'] == 'testharness:' + target['url'] and target['id'] not in local, 'duplicate or malformed target')
            local.add(target['id'])
            for field in ('path', 'url'):
                require(target[field] == row[field], 'target scope mismatch: ' + field)
            require(target['source_sha256'] == row['sha256'] == sources.get(row['path']), 'target source/helper identity mismatch')
            require((target['global'], target['variant']) == global_variant(row), 'target global/variant mismatch')
            expected = target['expected_subtests']
            require(isinstance(expected, list) and expected and all(isinstance(x, str) and x for x in expected)
                    and len(set(expected)) == len(expected) and expected.count(name) == 1, 'expected assertion population ambiguous')
            require(isinstance(target['profiles'], dict) and target['profiles'] and all(isinstance(k, str) and k and is_digest(v) for k,v in target['profiles'].items()), 'exact profile selectors required')
            known_profiles = self.candidate.get('core_profiles', {})
            # The caller supplies the verified registry members, never a free
            # per-test profile selection. Unknown policy selectors fail closed.
            require(known_profiles and all(known_profiles.get(k) == v for k,v in target['profiles'].items()), 'decision profile absent from verified registry')
            require(isinstance(target['platforms'], list) and target['platforms'] and len(set(target['platforms'])) == len(target['platforms'])
                    and set(target['platforms']) <= {'macOS', 'Linux', 'Windows'}, 'explicit platform selectors required')
            for profile in target['profiles']:
                for platform in target['platforms']:
                    key = (target['id'], profile, platform, name)
                    require(key not in self.targets, 'overlapping disposition decisions')
                    self.targets[key] = (decision, target, payload_hash)
        self.decisions[identifier] = decision

    def load_applications(self, path):
        path = Path(path).resolve()
        self.applications_sha256 = digest(path)
        self.track({'path': str(path), 'sha256': self.applications_sha256})
        value = json.loads(path.read_text())
        keys(value, ('schema_version', 'policy_sha256', 'applications'), 'applications')
        require(type(value['schema_version']) is int and value['schema_version'] == 1
                and value['policy_sha256'] == self.sha256 and isinstance(value['applications'], list), 'application policy/schema mismatch')
        for item in value['applications']:
            expected = (*IDENTITY_FIELDS, 'id', 'report_sha256', 'decision_id', 'decision_sha256',
                        'policy_sha256', 'assertion_name', 'raw_status', 'message_sha256',
                        'source_files', 'prerequisite_review_sha256', 'operator', 'reviewer',
                        'review', 'mechanism_evidence')
            keys(item, expected, 'disposition application')
            require(item['policy_sha256'] == self.sha256 and item['decision_id'] in self.decisions, 'unknown application decision/policy')
            require(item['raw_status'] == 'FAIL' and is_digest(item['message_sha256']) and is_digest(item['report_sha256']), 'application raw assertion invalid')
            require(text(item['operator'], 'application operator').strip().casefold() != text(item['reviewer'], 'application reviewer').strip().casefold(), 'application self-review')
            evidence = item['mechanism_evidence']
            require(isinstance(evidence, list) and evidence, 'application mechanism evidence missing')
            require(len({str(self.track(x)) for x in evidence}) == len(evidence), 'duplicate mechanism evidence')
            review = json.loads(self.track(item['review']).read_text())
            keys(review, ('schema_version', 'kind', 'application_sha256', 'operator', 'reviewer', 'approved', 'observations'), 'application review')
            require(type(review['schema_version']) is int and review['schema_version'] == 1 and review['kind'] == 'reviewed_disposition_application'
                    and review['approved'] is True and review['operator'] == item['operator'] and review['reviewer'] == item['reviewer']
                    and review['application_sha256'] == object_digest({k:v for k,v in item.items() if k != 'review'}), 'application independent review mismatch')
            text(review['observations'], 'independent application observations')
            key = (item['report_sha256'], item['id'], item['assertion_name'])
            require(key not in self.applications, 'duplicate or overlapping application')
            self.applications[key] = item
        self.recheck()
        return self

    def apply(self, row, result, report_hash, review, prerequisite_errors, report_context):
        identity = profile_identity(report_context, self.context['candidate_sha256'])
        require(all(identity[k] == self.context[k] for k in ('registry_sha256','profile_id','profile_sha256','platform')), 'cross-profile disposition application')
        applicable = [(d,t,h) for (identifier,profile,platform,_),(d,t,h) in self.targets.items()
                      if identifier == row['id'] and profile == identity['profile_id'] and platform == identity['platform']]
        supplied = [key for key in self.applications if key[:2] == (report_hash,row['id'])]
        if not applicable:
            require(not supplied, 'unused application without applicable decision')
            return []
        require(not prerequisite_errors, 'reviewed disposition cannot excuse prerequisites')
        require(row['test_type'] == 'testharness' and result.get('status') == 'OK', 'reviewed disposition cannot excuse harness failure')
        subtests = result.get('subtests')
        require(isinstance(subtests,list) and subtests and all(isinstance(s,dict) for s in subtests), 'missing disposition assertions')
        names = [s.get('name') for s in subtests]
        require(all(isinstance(n,str) for n in names) and len(set(names)) == len(names), 'ambiguous duplicate assertion names')
        raw = {s['name']:s for s in subtests}
        covered = set(); receipts = []
        for decision,target,payload_hash in applicable:
            require(names == target['expected_subtests'], 'expected assertion population changed or missing')
            expected = decision['assertion']; name = expected['name']; assertion = raw[name]
            require(assertion.get('status') == 'FAIL' and message_digest(assertion.get('message')) == expected['message_sha256'], 'expected reviewed FAIL not observed exactly')
            key=(report_hash,row['id'],name); item=self.applications.get(key)
            require(item is not None and key not in self.consumed, 'missing or multiply consumed disposition application')
            require(all(item.get(k) == v for k,v in identity.items()), 'application candidate/profile/envelope/run mismatch')
            require(item['decision_id'] == decision['id'] and item['decision_sha256'] == payload_hash
                    and item['message_sha256'] == expected['message_sha256'] and item['source_files'] == decision['source_files']
                    and item['prerequisite_review_sha256'] == object_digest(review), 'application source/assertion/review mismatch')
            require(name not in covered, 'overlapping assertion disposition')
            covered.add(name); self.consumed.add(key)
            receipts.append({'decision_id':decision['id'],'decision_sha256':payload_hash,'policy_sha256':self.sha256,
                             'application_sha256':object_digest(item),'applications_sha256':self.applications_sha256,'assertion_name':name})
        require(all(s.get('status') == ('FAIL' if s['name'] in covered else 'PASS') for s in subtests), 'unaffected assertion did not independently pass')
        require(set(supplied) == {(report_hash,row['id'],name) for name in covered}, 'unused or extra application')
        return receipts

    def finish(self):
        require(set(self.applications) == self.consumed, 'unused disposition applications remain')
        self.recheck()

    def recheck(self):
        for original, (expected, destination) in self.bindings.items():
            require(str(Path(original).resolve()) == destination, 'disposition artifact destination changed')
        for path, expected in self.inputs.items():
            require(Path(path).is_file() and digest(path) == expected, 'disposition input changed: ' + path)


def load_policy(path, *, candidate, coverage, checkout, profile_context):
    return Policy(path, candidate=candidate, coverage=coverage, checkout=checkout, profile_context=profile_context)
