# Oryn source patches

Base: crates.io harfrust 0.12.0, copied from the installed registry source with
upstream license files retained. Local changes recognize x-hbov- followed by
exactly eight hex digits as a lossless four-byte OpenType language override from
Parley, with precedence over ordinary language inference. Source generation only:
builds, upstream tests and conformance verification remain deferred.
