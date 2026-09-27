"""Reconcile one original's observations without confusing them with qualification.

This module never loads historical runs or treats a harness OK as subtest PASS.
Admission, protocol custody, and independent review remain separate gates.
"""
from collections import Counter
import json
from pathlib import Path


def reconcile_wpt(expected_urls, events, *, execution, assessment_total=752):
    """Return counts and discrepancies for exactly one run's mozlog events.

    An ordinary exit 1 can contain complete observations with genuine failures.
    Missing/duplicate events or failed cleanup cannot become a valid original.
    """
    expected = list(expected_urls)
    if (not expected or any(not isinstance(url, str) or not url.startswith('/') for url in expected)
            or len(expected) != len(set(expected))):
        raise ValueError('expected URLs must be a nonempty unique URL population')
    if type(assessment_total) is not int or assessment_total < 0:
        raise ValueError('assessment total must be a nonnegative integer')
    selected = set(expected)
    starts, ends, outcomes, subtests = Counter(), Counter(), Counter(), Counter()
    discrepancies = []
    suite_starts = suite_ends = 0
    active_suite = False
    for number, event in enumerate(events, 1):
        if not isinstance(event, dict):
            raise ValueError('mozlog event must be an object at event ' + str(number))
        action = event.get('action')
        if action == 'suite_start':
            suite_starts += 1
            if active_suite or suite_starts > 1:
                discrepancies.append('multiple suite starts: one original required')
            active_suite = True
        elif action == 'suite_end':
            suite_ends += 1
            if not active_suite:
                discrepancies.append('suite end without active suite')
            active_suite = False
        elif action in ('test_start', 'test_end', 'test_status'):
            url = event.get('test')
            if not isinstance(url, str):
                raise ValueError('test event lacks URL at event ' + str(number))
            if not active_suite:
                discrepancies.append('test event outside suite: ' + url)
            if url not in selected:
                discrepancies.append('unexpected URL: ' + url)
            if action == 'test_start':
                starts[url] += 1
            elif action == 'test_end':
                ends[url] += 1
                if starts[url] != 1 or ends[url] != 1:
                    discrepancies.append('end without unique preceding start: ' + url)
                status = event.get('status')
                if not isinstance(status, str) or not status:
                    raise ValueError('test end lacks status: ' + url)
                outcomes[status] += 1
            else:
                if starts[url] != 1 or ends[url] != 0:
                    discrepancies.append('subtest outside unique active test: ' + url)
                status = event.get('status')
                if not isinstance(status, str) or not status:
                    raise ValueError('subtest lacks status: ' + url)
                subtests[status] += 1
    missing_starts = sorted(selected - starts.keys())
    missing_ends = sorted(selected - ends.keys())
    duplicate_starts = sorted(url for url, count in starts.items() if count != 1)
    duplicate_ends = sorted(url for url, count in ends.items() if count != 1)
    complete = (suite_starts == suite_ends == 1 and not active_suite and not discrepancies
                and not missing_starts and not missing_ends
                and not duplicate_starts and not duplicate_ends)
    custody = (type(execution.get('returncode')) is int and execution['returncode'] in (0, 1)
               and execution.get('timed_out') is False
               and execution.get('cleanup_complete') is True
               and execution.get('error') is None)
    return {
        'expected_automated': len(expected), 'assessment_total': assessment_total,
        'started': sum(starts.values()), 'completed': sum(ends.values()),
        'unique_completed': len(selected & ends.keys()),
        'outcomes': dict(sorted(outcomes.items())), 'subtest_outcomes': dict(sorted(subtests.items())),
        'missing_starts': missing_starts, 'missing_ends': missing_ends,
        'duplicate_starts': duplicate_starts, 'duplicate_ends': duplicate_ends,
        'discrepancies': discrepancies, 'observations_complete': complete,
        'process_custody_complete': custody,
        'compatibility_complete': complete and custody,
        'qualification': 'not-evaluated',
    }


def read_mozlog(path):
    """Stream the raw log; truncated/invalid JSON is a failure, never skipped."""
    with Path(path).open(encoding='utf-8') as stream:
        for number, line in enumerate(stream, 1):
            try:
                yield json.loads(line)
            except ValueError as error:
                raise ValueError('invalid mozlog JSON at line ' + str(number)) from error
