# Ontology title extraction patch

This is the complete crates.io package `pulldown-cmark` 0.13.4, under its original
MIT license. `ONTOLOGY-UPSTREAM.json` records its verified archive checksum and
every original file checksum. The workspace selects this local dependency through
`[patch.crates-io]`; the global Cargo cache remains unchanged.

Only `src/firstpass.rs`, `src/parse.rs` and `src/tree.rs` change upstream code.
The normal Parser constructors preserve the upstream grammar, events, byte
offsets, link payloads and Vec growth policy. Private tree node identifiers use
checked `NonZeroU32` values; byte offsets and public event ranges remain `usize`.
On 64-bit targets a node occupies 40 bytes rather than 48. Conversions and index
arithmetic reject overflow, underflow and zero instead of truncating or wrapping.
The native/importer source budgets are far below the four-billion-node range.

`first_h1_prefix` recognizes all original blocks and definitions, then parses the
prefix through the first root H1 with the complete definition context and the
original source length's reference expansion budget. It runs earlier inline
recognition and budget effects in their original order, internally discarding
events before the H1. Those discarded link payloads share one temporary slot.
Scratch mode ends before the H1; the returned H1 events and offsets match the
normal parser. Earlier inline nodes may be omitted only when the entire region
before the H1 has no literal `[`, so it cannot use link references or footnotes.
Unsupported options use the normal full parser.

`first_h1_title_events` adds a narrower event stream for the title renderer.
`new_title_events` provides the same stream over the complete document for the
first nonempty H1 fallback. Link and image tag metadata (type, URL, title, id) is
omitted in these two streams. Inline grammar, budget effects, visible Text/Code,
breaks, nesting and source ranges stay unchanged. These streams must not render
a complete document with URLs. The exact H1 event oracle remains separate from
the title stream's normalized event oracle; both run on the upstream corpus.

Title factories select fixed 8192-entry storage chunks for nodes and CowStr
payload slots before either pass or the complete-document fallback allocates.
Global numeric indices keep their original meaning across chunk boundaries.
Pop, reinsertion and clone preserve owned values; earlier live nodes are not
reclaimed. The normal Parser retains contiguous Vec storage. This avoids moving
a large live title arena during growth; reserved capacity is not an RSS claim.

Inline code in title storage keeps two checked compact raw inner source bounds.
The original line-ending, container-prefix, table-escape and padding rules run
only when each Code event is emitted, so normalized code strings do not accumulate
in the payload pool. A single normalization function serves both paths; normal
Parser constructors still normalize eagerly into their original payload pool.
The title node also preserves table context from code recognition, before
superscript, subscript or other inline ancestors are resolved. Delayed escape
normalization uses that original context rather than reclassifying the later
event spine. Bounds that cannot fit use that same original path. Public event ranges remain
`usize`; Event::Code values and offsets match the normal parser. No source bytes,
public offsets or database columns are narrowed.

Ontology's exact reader owns this patch. Public upstream remains authoritative
for unchanged Markdown grammar. This directory does not define native access
policy, database ownership or caller permissions. Complete original SHA checks,
metadata validation, redaction and atomic response limits remain with the
existing native adapter and context-core validators. The original database
column types and bytea storage are unchanged.

For an upstream update, verify the new archive, refresh all original checksums,
replay the three-file patch, and run the upstream suite, both title differential
oracles, packed-index boundary tests and native exact-reader integration tests.
The existing `scripts/verify.sh` includes the upstream suite. Performance claims
must compare complete outputs and bind measured binaries to checked source.
