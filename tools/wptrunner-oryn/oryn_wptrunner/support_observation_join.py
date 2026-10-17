"""Trusted supervisor join; never executes callbacks or invents test results.

Inputs are the authenticated native response snapshot and the original upstream
executor conversion. A caller-supplied URL/hash/assertion list cannot issue the
native pinned descriptor. This module does not mutate the original outcomes.
"""
from collections import Counter
import hashlib
import json

PARENT = '/IndexedDB/idb-partitioned-coverage.sub.html'
CHILD = '/IndexedDB/resources/idb-partitioned-coverage-iframe.html'
REVISION = '719d5e38fdd0903a18ed9007aba816c98cc491e0'
PARENT_HASH = '9767126059bb9dea9a4e59b9a4f01a2a2f439d966d8c57a1e9f97104576afff0'
CHILD_HASH = 'ba2ed49fd6b15b72e6ac2006bb61ba95fad7132d54668557c3188892891c0bc6'
PINNED_IMPORTS = [
    {'path': '/resources/testharness.js', 'sha256': 'd2399236c2a09c429804ff2299ad6629e17e2b53f17a74dd341e936adb11ae3e'},
    {'path': '/IndexedDB/resources/support.js', 'sha256': '3bbc4fa7b1a4e2354a05322b4fd4e3ac1afad6ad065af746d9fc377ee28ef1a7'},
    {'path': '/IndexedDB/resources/nested-cloning-common.js', 'sha256': '579c2829a8ed271c6baa0e66c80660bb5b39cb01914637b4dfdc0170700d0f07'},
    {'path': '/IndexedDB/resources/support-get-all.js', 'sha256': '377019848bf1ef23e74a90470a5ddfcbf55e3ff82c35be0ebd3e9c7e4e443444'},
    {'path': '/IndexedDB/resources/support-promises.js', 'sha256': 'b17a508df2752a4240e898013fd47313e4742b6f195d3e1342fbb7134d5948fb'},
    {'path': '/IndexedDB/idbindex_getAll.any.js', 'sha256': '3b6bcf5a4d021a420d3d8a30ef5803cee398e3312bde589e25a43b9c7647906d'},
]
PINNED_NAMES = [
    'Deletes are processed in order', ' - advance', ' - continue',
    ' - fresh advance still async', ' - fresh continue still async',
    'Single item get', 'Empty object store', 'Get all', 'Get all with generated keys',
    'Get all with large values', 'maxCount=10', 'Get bound range',
    'Get bound range with maxCount', 'Get upper excluded', 'Get lower excluded',
    'Get bound range (generated) with maxCount', 'Non existent key', 'maxCount=0',
    'Max value count', 'Query with empty range where  first key < upperBound',
    'Query with empty range where lowerBound < last key', 'Retrieve multiEntry key',
    'Retrieve one key multiple values', 'Get all values with invalid query keys',
    'IDBObjectStore.openKeyCursor() - forward iteration',
    'IDBObjectStore.openKeyCursor() - reverse iteration',
    'IDBObjectStore.openKeyCursor() - forward iteration with range',
    'IDBObjectStore.openKeyCursor() - reverse iteration with range',
    'IDBObjectStore.openKeyCursor() - invalid inputs',
    ' - request gotten by the handler', ' - request returned by open()',
]

def digest_object(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'),
                                    ensure_ascii=False).encode()).hexdigest()

def _hash(value):
    return isinstance(value, str) and len(value) == 64 and all(c in '0123456789abcdef' for c in value)

def _owner(value):
    fields = {'page', 'occurrence', 'document_generation', 'stream_epoch', 'authority_epoch'}
    return (isinstance(value, dict) and set(value) == fields
            and all(type(value[k]) is int and value[k] >= 0 for k in fields)
            and value['occurrence'] > 0)

def join(descriptor, native, parent_status, subtests, *, candidate, process_complete):
    """Produce an observation, not qualification. Parent failure dominates.

    Descriptor comes from the hash-bound fixture contract, not the child DOM.
    Native snapshot is collected under current parent/child ownership. Imported
    subtests come from unchanged pinned testharness fetch_tests_from_window.
    Exact ordered names, including duplicate occurrences, are descriptor-pinned.
    """
    result = {'schema': 1, 'kind': 'parent-derived-support-observation',
              'parent': PARENT, 'support': CHILD, 'complete': False,
              'qualification': 'not-evaluated'}
    required = {'schema', 'revision', 'parent', 'parent_sha256', 'child',
                'child_sha256', 'imports', 'assertions'}
    if (not isinstance(descriptor, dict) or set(descriptor) != required
            or descriptor['schema'] != 1 or descriptor['revision'] != REVISION
            or descriptor['parent'] != PARENT or descriptor['child'] != CHILD
            or descriptor['parent_sha256'] != PARENT_HASH
            or descriptor['child_sha256'] != CHILD_HASH):
        raise ValueError('unbound pinned support descriptor')
    imports = descriptor['imports']
    if (not isinstance(imports, list) or not imports
            or any(not isinstance(row, dict) or set(row) != {'path', 'sha256'}
                   or not isinstance(row['path'], str) or not row['path'].startswith('/')
                   or not _hash(row['sha256']) for row in imports)
            or len({row['path'] for row in imports}) != len(imports)):
        raise ValueError('invalid pinned source-import closure')
    names = descriptor['assertions']
    if imports != PINNED_IMPORTS or names != PINNED_NAMES:
        raise ValueError('descriptor differs from pinned full source and assertion closure')
    result['descriptor_sha256'] = digest_object(descriptor)
    # Preserve a real parent crash/error even if stale child rows were delivered.
    if parent_status != 'OK' or process_complete is not True:
        result['reason'] = 'parent-or-process-incomplete'
        return result
    if (not _hash(candidate) or not isinstance(native, dict)
            or native.get('schema') != 1 or native.get('candidate') != candidate
            or native.get('descriptor') != result['descriptor_sha256']
            or native.get('current') is not True
            or not _owner(native.get('parent')) or not _owner(native.get('child'))
            or native['parent']['page'] != native['child']['page']
            or native.get('body_sha256') != CHILD_HASH
            or native.get('eof') is not True
            or native.get('imports') != imports):
        result['reason'] = 'native-response-or-owner-closure-incomplete'
        return result
    rows = list(subtests)
    if any(not isinstance(row, dict) or set(row) != {'name', 'status'}
           or not isinstance(row['name'], str) or not isinstance(row['status'], str) for row in rows):
        raise ValueError('invalid original upstream subtest observation')
    observed = [row['name'] for row in rows]
    # Original remote-state registration normally retains declaration order,
    # but identity is name plus occurrence, not async completion position.
    if Counter(observed) != Counter(names):
        result['reason'] = 'assertion-identity-or-multiplicity-mismatch'
        result['missing'] = list((Counter(names) - Counter(observed)).elements())
        result['unexpected'] = list((Counter(observed) - Counter(names)).elements())
        return result
    result['assertions'] = [{'name': row['name'], 'occurrence': sum(
        old['name'] == row['name'] for old in rows[:index]), 'status': row['status']}
        for index, row in enumerate(rows)]
    result['native'] = native
    result['complete'] = True
    result['all_pass'] = all(row['status'] == 'PASS' for row in rows)
    return result
