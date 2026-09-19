"""External product for the repository-pinned WPT runner.

The native command server is required. The diagnostic whole-test JSONL runner
cannot satisfy this product's command-level WebDriver executor contract.
"""

WPT_REVISION = "719d5e38fdd0903a18ed9007aba816c98cc491e0"


def load_product():
    # Import only when upstream asks for the product, in its dependency runtime.
    from .product import product
    return product()
