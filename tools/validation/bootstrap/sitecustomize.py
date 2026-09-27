"""Opt-in typed fixture bootstrap, inherited by upstream multiprocessing spawn."""
import os

if any(os.environ.get(name) for name in ('ORYN_FIXTURE_TLS_CONTRACT',
        'ORYN_FIXTURE_TLS_CONTRACT_SHA256', 'ORYN_FIXTURE_TLS_RECEIPTS')):
    try:
        from oryn_validation.fixture_tls import install
        install()
    except BaseException as error:
        import sys
        sys.stderr.write('fatal typed fixture TLS bootstrap: ' + repr(error) + '\n')
        sys.stderr.flush()
        os._exit(78)
