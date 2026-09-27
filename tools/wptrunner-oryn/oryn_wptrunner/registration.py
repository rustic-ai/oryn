"""Verify upstream's normal plugin discovery selects this frozen source export.

The adjacent committed dist-info makes the source package discoverable without
installing a second copy into the immutable Python environment. No upstream
registry or metadata discovery function is replaced.
"""
from importlib import metadata
from pathlib import Path
import tomllib


def verify_registration(source_root):
    root = Path(source_root).absolute() / 'tools/wptrunner-oryn'
    project = tomllib.loads((root / 'pyproject.toml').read_text())['project']
    declared = project['entry-points']['wptrunner.products']
    if declared != {'oryn': 'oryn_wptrunner:load_product'}:
        raise ValueError('source declares an unexpected Oryn product entry point')
    # entry_points() and upstream's mapping deduplicate distribution/product
    # names. Inspect raw distributions first so a hidden second copy fails too.
    candidates = []
    distributions = []
    for dist in metadata.distributions():
        name = (dist.metadata.get('Name') or '').lower().replace('_', '-').replace('.', '-')
        if name == project['name']:
            distributions.append(dist)
        candidates.extend(ep for ep in dist.entry_points
                          if ep.group == 'wptrunner.products' and ep.name == 'oryn')
    if len(distributions) != 1 or len(candidates) != 1:
        raise ValueError('Oryn product requires exactly one source distribution and entry point')
    dist, entry = distributions[0], candidates[0]
    if (dist.version != project['version'] or entry.value != declared['oryn']
            or entry.dist.metadata.get('Name') != project['name']
            or Path(dist.locate_file('')).absolute() != root
            or Path(entry.dist.locate_file('')).absolute() != root):
        raise ValueError('Oryn product metadata differs from selected runner source')
    import oryn_wptrunner
    if Path(oryn_wptrunner.__file__).absolute() != root / 'oryn_wptrunner/__init__.py':
        raise ValueError('Oryn product module differs from selected runner source')
    from wptrunner.products import Product, get_all_products
    selected = get_all_products().get('oryn')
    if (selected is None or selected.value != entry.value
            or Path(selected.dist.locate_file('')).absolute() != root):
        raise ValueError('upstream selected a foreign Oryn product')
    product = Product.from_product_name('oryn')
    from . import product as module
    if (Path(module.__file__).absolute() != root / 'oryn_wptrunner/product.py'
            or product.get_browser_cls('testharness') is not module.OrynBrowser):
        raise ValueError('upstream loaded a foreign Oryn browser')
    return product
