"""Strict preparation marker; artifact freshness belongs to build assembly."""


def pending_rebind(profile):
    if 'rust_source_rebind_intermediate' not in profile:
        return None
    value = profile['rust_source_rebind_intermediate']
    fields = {'schema_version', 'old_profile_sha256', 'old_rust_tree', 'new_rust_tree',
              'fresh_snapshot_required', 'fresh_worker_required', 'qualification_complete'}
    if (not isinstance(value, dict) or set(value) != fields or value['schema_version'] != 1
            or type(value['schema_version']) is not int
            or type(value['fresh_snapshot_required']) is not bool
            or type(value['fresh_worker_required']) is not bool
            or value['qualification_complete'] is not False
            or not all(isinstance(value[key], str) and len(value[key]) == 64
                       and all(c in '0123456789abcdef' for c in value[key])
                       for key in ('old_profile_sha256', 'old_rust_tree', 'new_rust_tree'))):
        raise ValueError('malformed Rust source rebind intermediate')
    return dict(value)
