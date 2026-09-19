"""Candidate-bound execution of the required external WebM decoder regression."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

from completion_execution import ROOT, persist, reference, run_owned, source_hash
from completion_provenance import load_candidate, read_record
from generation_gate import generation_gate, GenerationLease
from native_protocol_versions import runtime_matches
from wpt_full_qualification import REVISION

TEST = 'media_capture::url_asset::decoder_tests::pinned_vp9_opus_decodes_pixels_pcm_seek_and_pip_with_owner_cleanup'
FIXTURES = ('media/movie_300.webm', 'media/rgb100.webm') + tuple(
    'mimesniff/media/resources/' + name for name in (
        'mp3-raw.mp3', 'mp3-with-id3.mp3', 'flac.flac', 'ogg.ogg', 'mp4.mp4', 'wav.wav', 'webm.webm'
    )
)


def media_source_hash(root=ROOT):
    base = Path(root) / 'crates/oryn-native'
    paths = ['../../Cargo.toml', '../../Cargo.lock', 'build.rs', 'Cargo.toml', 'src/media_capture.rs']
    paths += [item.relative_to(base).as_posix() for item in (base / 'src/media_capture').iterdir() if item.is_file()]
    vendor = Path(root) / "vendor/libvpx"
    if not (vendor / "LICENSE").is_file():
        raise ValueError("pinned libvpx source is missing")
    paths += ["../../vendor/libvpx/" + item.relative_to(vendor).as_posix() for item in vendor.rglob("*") if item.is_file()]
    result = hashlib.sha256()
    for name in sorted(paths):
        path = name.encode()
        data = (base / name).read_bytes()
        result.update(len(path).to_bytes(8, 'big')); result.update(path)
        result.update(len(data).to_bytes(8, 'big')); result.update(data)
    return result.hexdigest()


def fixture_inputs(candidate):
    contract = json.loads(candidate['artifacts']['native_controls_contract'].read_text())
    checkout = Path(contract['upstream_checkout']).resolve()
    revision = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    if revision != REVISION:
        raise ValueError('media fixture checkout is not the pinned WPT revision')
    records = []
    for name in FIXTURES:
        path = (checkout / name).resolve()
        if checkout not in path.parents:
            raise ValueError('media fixture escaped checkout')
        original = subprocess.check_output(['git', '-C', str(checkout), 'show', REVISION + ':' + name])
        record = reference(path)
        if record['sha256'] != hashlib.sha256(original).hexdigest():
            raise ValueError('media fixture differs from pinned Git blob: ' + name)
        records.append(record)
    return checkout, records


def validate_output(text, expected_source):
    builds = re.findall(r'^ORYN_MEDIA_BUILD (\{.*\})$', text, re.MULTILINE)
    if len(builds) != 1:
        raise ValueError('native media regression has no unique compiled build identity')
    build = json.loads(builds[0])
    if build.get('media_source_sha256') != expected_source:
        raise ValueError('native media executable was compiled from another media source')
    if not runtime_matches(build.get('compiled_protocols', {}), ROOT):
        raise ValueError('native media executable protocol identity differs')
    if not re.search(r'^test ' + re.escape(TEST) + r' \.\.\.(?: ok)?\s*$', text, re.MULTILINE):
        raise ValueError('required native media fixture regression did not pass')
    if not re.search(r'^test result: ok\. 1 passed; 0 failed; 0 ignored;', text, re.MULTILINE):
        raise ValueError('native media run omitted or ignored its required regression')
    return build


def qualify(candidate_path, gate, output):
    admission = GenerationLease(gate, validator=generation_gate)
    candidate = load_candidate(candidate_path)
    if source_hash() != candidate['record']['source_tree_sha256']:
        raise ValueError('media source differs from frozen candidate')
    checkout, fixtures = fixture_inputs(candidate)
    binary = candidate['artifacts']['native_media_tests']
    argv = [str(binary), '--exact', TEST, '--ignored', '--nocapture', '--test-threads=1']
    output = Path(output).resolve(); admission.check(); output.mkdir(parents=True, exist_ok=False)
    before = dict(candidate=reference(candidate_path), generation_gate=reference(gate),
                  binary=reference(binary), fixtures=fixtures, checkout=str(checkout),
                  media_source_sha256=media_source_hash(), argv=argv)
    persist(output / 'preflight.json', before)
    errors = []
    try:
        generation_gate(gate)
        for field in ('candidate', 'generation_gate', 'binary'): read_record(before[field])
        for fixture in fixtures: read_record(fixture)
        if source_hash() != candidate['record']['source_tree_sha256']:
            raise ValueError('media source changed before execution')
        # The caller supplies neither the test filter nor fixture environment.
        admission.check()
        execution = run_owned(argv, output, timeout=600,
                              environment=admission.environment(dict(os.environ, ORYN_WPT_ROOT=str(checkout))))
        persist(output / 'execution.json', execution)
        if execution['returncode'] != 0 or execution['timed_out'] or execution['error'] or not execution['cleanup_complete']:
            raise ValueError('native media execution or cleanup failed')
        validate_output((output / 'stdout.log').read_text(), before['media_source_sha256'])
    except Exception as error:
        errors.append(str(error))
    try:
        generation_gate(gate)
        for field in ('candidate', 'generation_gate', 'binary'): read_record(before[field])
        if fixture_inputs(candidate) != (checkout, fixtures) or media_source_hash() != before['media_source_sha256'] or source_hash() != candidate['record']['source_tree_sha256']:
            raise ValueError('media candidate or fixture changed during execution')
        if load_candidate(candidate_path)['sha256'] != candidate['sha256']:
            raise ValueError('media candidate changed')
    except Exception as error:
        errors.append(str(error))
    result = dict(schema_version=1, candidate_manifest_sha256=candidate['sha256'],
                  preflight=reference(output / 'preflight.json'), errors=errors, qualified=not errors)
    persist(output / 'result.json', result)
    return result


def validate_result(raw, candidate):
    raw = Path(raw)
    result = json.loads((raw / 'result.json').read_text())
    if read_record(result['preflight']) != (raw / 'preflight.json').resolve():
        raise ValueError('native media preflight is outside its original run')
    before = json.loads(read_record(result['preflight']).read_text())
    if result.get('schema_version') != 1 or result.get('candidate_manifest_sha256') != candidate['sha256'] or result.get('qualified') is not True or result.get('errors'):
        raise ValueError('native media result is unqualified or belongs to another candidate')
    if read_record(before['candidate']) != candidate['path'] or read_record(before['binary']) != candidate['artifacts']['native_media_tests']:
        raise ValueError('native media candidate artifact substituted')
    read_record(before['generation_gate'])
    checkout, fixtures = fixture_inputs(candidate)
    if before['checkout'] != str(checkout) or before['fixtures'] != fixtures or before['media_source_sha256'] != media_source_hash():
        raise ValueError('native media source or fixture inventory changed')
    argv = [str(candidate['artifacts']['native_media_tests']), '--exact', TEST, '--ignored', '--nocapture', '--test-threads=1']
    execution = json.loads((raw / 'execution.json').read_text())
    if before['argv'] != argv or execution['argv'] != argv or execution['returncode'] != 0 or execution['timed_out'] is not False or execution['error'] is not None or execution['cleanup_complete'] is not True:
        raise ValueError('native media execution receipt differs')
    validate_output((raw / 'stdout.log').read_text(), before['media_source_sha256'])
    return result
