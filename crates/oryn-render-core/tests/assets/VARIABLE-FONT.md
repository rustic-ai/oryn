# Variable font regression fixture

Copied unchanged from WPT revision `719d5e38fdd0903a18ed9007aba816c98cc491e0`,
`css/css-fonts/matching/resources/variabletest_matching.ttf`.

SHA-256: `79252fe83b1d5092be1599fbd04ae75c78252114b89a858adbbf7768a0c82184`.

See WPT-LICENSE.md for the retained upstream license. The upstream matching
README defines P at weight 100 as matching /, and P at weight 900 as matching
comma; these static reference glyphs provide independent outline expectations.

This render-core fixture was copied without font-byte changes from
`crates/oryn-render/tests/assets/variabletest_matching.ttf` when the shared
renderer moved into `oryn-render-core`. Both crates retain their own fixture
and the upstream license so crate-local tests do not depend on sibling test files.
