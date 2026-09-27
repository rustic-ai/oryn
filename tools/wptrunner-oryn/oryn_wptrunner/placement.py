"""Read-only native diagnostic capture after unchanged upstream conversion."""
import json
import os
from pathlib import Path
from .provenance import load_run_contract as load_contract

TEST = '/__oryn_shared_memory__/shared-memory/engine-lifecycle.https.html?placement=1'


def configuration():
    path = os.environ.get('ORYN_WPT_PLACEMENT_CONTEXT')
    if not path:
        return None
    path = Path(path)
    config = json.loads(path.read_text())
    bound = load_contract(config['run_contract'])
    if (config.get('schema') != 1 or config.get('test') != TEST
            or config.get('candidate_sha256') != bound['candidate_sha256']
            or config.get('run_contract_sha256') != bound['sha256']):
        raise ValueError('placement context does not match verified candidate')
    nonce = config.get('nonce', '')
    if len(nonce) != 64 or any(char not in '0123456789abcdef' for char in nonce):
        raise ValueError('invalid placement nonce')
    output = Path(config['output'])
    if not path.is_absolute() or not output.is_absolute() or output.parent != path.parent:
        raise ValueError('placement artifacts must share explicit execution directory')
    if bound.get('typed_run') is not None:
        config['_typed_artifacts'] = bound['candidate']['artifacts']
    return config


def capture(executor, test, config):
    if test.url != config['test']:
        return
    record = {key: config[key] for key in ('schema', 'test', 'nonce', 'candidate_sha256', 'run_contract_sha256')}
    try:
        record['snapshot'] = executor.protocol.webdriver.send_session_command(
            'POST', 'oryn/diagnostics/placement', {'nonce': config['nonce']})
    except Exception as error:
        # Diagnostic failure cannot rewrite the real upstream test result.
        record['error'] = type(error).__name__ + ': ' + str(error)
    if '_typed_artifacts' in config:
        from oryn_validation.live_six import capture as capture_live_six
        try:
            live_six, confirmed = capture_live_six(record['snapshot'], config, config['_typed_artifacts'],
                Path(config['output']).parent, diagnostic_reader=lambda: executor.protocol.webdriver.send_session_command(
                    'POST', 'oryn/diagnostics/placement', {'nonce': config['nonce']}))
            record['snapshot'] = confirmed
        except Exception as error:
            live_six = {'schema_version': 1, 'kind': 'live-six-native-dedicated-workers',
                        'error': type(error).__name__ + ': ' + str(error)}
        with Path(config['output']).with_name('live-six.json').open('x') as stream:
            json.dump(live_six, stream, sort_keys=True, indent=2)
            stream.write('\n')
    with Path(config['output']).open('x') as stream:
        json.dump(record, stream, sort_keys=True, indent=2)
        stream.write('\n')
