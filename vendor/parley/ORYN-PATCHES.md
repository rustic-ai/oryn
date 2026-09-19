# Oryn source patches

Base: crates.io parley 0.11.1, copied from the installed registry source with
upstream license files retained. Local changes add an exact four-byte OpenType
language override to style resolution and shaping itemization, and preserve the
full canonical language/script/region locale passed to Harfrust. The local
Harfrust hbov extension transports the exact override bytes. Source generation
only: builds, upstream tests and conformance verification remain deferred.

Builders additionally expose paragraph BaseDirection, reset for each new builder.
Analysis passes explicit levels even for ASCII text, so neutral ordering and
paragraph alignment can follow the caller's base direction independently of the
first strong character. Auto retains upstream inference behavior.

IndentOptions includes skip_first_line for independently shaped bidi paragraphs
within one CSS formatting block. It preserves ordinary first-line, each-line and
hanging indentation across those boundaries.

AlignmentOptions includes last_line for CSS text-align-last. Justification records
per-line space adjustments/counts, updates measured advances and widths, and
undoes recorded adjustments before realignment or line breaking, including when
only the final line was justified.

TabSize supports space counts and absolute lengths. Font-instance space metrics
resolve stops; line-break checkpoints retain per-tab advances, committed runs
transport those widths into glyph iteration, hit testing and geometry, and tabs
paint the font's space glyph. Position includes line origin and indentation.

LineWhitespace distinguishes collapsed, preserved and hanging line-edge spaces.
Per-line cluster advances now also record zero-width collapsed spaces; trailing
space handling spans multiple styled runs, leaves NBSP measured, and excludes
trimmed spaces from justification and painted glyphs.

LineWhitespace::BreakSpaces adds a wrapping opportunity after every preserved
space/tab, including across run boundaries. It bypasses ordinary overflow-space
hanging, keeps forced breaks, and respects nowrap while retaining space widths.

LineWhitespace::ConditionalHang retains trailing spaces within available width
at forced breaks and paragraph end, and hangs the full sequence at soft wraps.
Oryn uses this mode for pre-wrap; the upstream default Hang remains unconditional.

Overflowing collapsible/hanging space and tab sequences are retained until the
following content or mandatory break establishes the line ending. NBSP no longer
uses the overflowing-space shortcut; break-spaces retains individual opportunities.

Unicode space separators have separate cluster classification, allowing trailing
separator sequences to hang without deleting their glyph/source advances. Terminal
ASCII spaces collapse before hanging the rest of a mixed sequence; Unicode line
boundaries continue to determine legal breaks (including glue space characters).

Intrinsic width measurement now uses an isolated clone and the shared line breaker
at unbounded and zero available widths. It restores natural justification,
excludes break-word emergency opportunities and hanging space contributions, and
preserves the caller's measured/positioned layout. Allocation/performance
qualification remains deferred alongside the rest of the unbuilt generation.
