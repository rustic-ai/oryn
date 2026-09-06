# Oryn XML adapter patches

Base: xml 1.4.0, MIT, https://github.com/kornelski/xml-rs.
Source copied from the Cargo registry package; license retained.

- Opt-in preservation of namespace declaration attributes for native DOM Attr nodes.
- Check duplicate namespace declarations before stripping declaration attributes.
- Enforce reserved XML/XMLNS namespace bindings and forbid xmlns element prefixes.
- Permit relative namespace names `xml` and `xmlns` as default namespace URI values.

Oryn uses one document root, preserves comments and CDATA, and retains the library's
entity expansion bounds. External entity loading is not implemented. DTD attribute
defaults and validation remain gaps; this integration does not claim full XML conformance.
