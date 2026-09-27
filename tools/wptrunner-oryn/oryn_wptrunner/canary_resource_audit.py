"""Retain one real wptserve negative-control response without broad debug logs."""
import re

MISSING_HELPER = '/__oryn_qualification_missing_helper__.js'
_REQUEST = re.compile(r"^[^\r\n]+ - - \[[^\]\r\n]+\] ['\"]GET /__oryn_qualification_missing_helper__\.js HTTP/1\.[01]['\"] ([1-5][0-9]{2}) (?:[0-9]+|-)$")

# Pinned wptserve's normal Response writer bypasses BaseHTTPRequestHandler's
# log_request. finish_handling logs status/method/path/referer/body length instead.
_RESPONSE = re.compile(r"^([1-5][0-9]{2}) GET /__oryn_qualification_missing_helper__\.js \([^\r\n]*\) [0-9]+$")


def response_status(record):
    if (record.get('action') != 'log' or record.get('component') != 'wptserve'
            or record.get('level') not in ('DEBUG', 'INFO')):
        return None
    message = record.get('message')
    if not isinstance(message, str) or len(message) > 4096:
        return None
    match = _REQUEST.fullmatch(message) or _RESPONSE.fullmatch(message)
    return int(match.group(1)) if match else None


class RequestEvidenceFilter:
    def __init__(self, previous):
        self.previous = previous

    def __call__(self, record):
        # Preserve the original status/request text; only admit this otherwise
        # filtered DEBUG observation to the existing INFO raw-log pipeline.
        candidate = dict(record)
        candidate.setdefault('component', 'wptserve')
        if response_status(candidate) is not None and record['level'] == 'DEBUG':
            record = dict(candidate, level='INFO')
        return self.previous(record) if self.previous is not None else record


class RequestEvidenceScope:
    def __init__(self, logger, config):
        self.logger = logger
        self.config = config
        self.previous_filter = logger.component_filter
        self.previous_level = config.logging['level']
        self.filter = RequestEvidenceFilter(self.previous_filter)
        self.closed = False
        config.logging['level'] = 'DEBUG'
        try:
            logger.component_filter = self.filter
        except BaseException:
            config.logging['level'] = self.previous_level
            raise

    def close(self):
        if self.closed:
            return
        self.logger.component_filter = self.previous_filter
        self.config.logging['level'] = self.previous_level
        self.closed = True
