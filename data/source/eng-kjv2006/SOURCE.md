# Source: KJV (1769), eBible.org `eng-kjv2006`

- URL: https://ebible.org/Scriptures/eng-kjv2006_usfm.zip
- Downloaded: 2026-10-10. Upstream source files dated 2026-09-26 (see `copr.htm`).
- Zip sha256: `5789dcd000d60a554abb4cf9b37ce10a53628bfc686e11ada2d6c981455239c1`
- License: Public Domain (outside the UK; see `copr.htm`). Text courtesy of CrossWire Bible Society and eBible.org.
- Contents: 66 protocanonical books, USFM with Strong's numbers. Vendored verbatim (only the `.usfm` files and
  `copr.htm`). `eng-kjv` (with Apocrypha) has identical protocanon text after stripping `\w` attributes.

This directory is the exact input of `text-build` for `kjv-1769-v1`. **Do not edit or refresh it**: the
tokenization (and therefore every user's tagging ordinals) is derived from these bytes, and the golden checks in
`fixtures/text/` will fail. A corrected text means a new `text_id` plus an ordinal remap migration.
