# Source: NET Bible, eBible.org `engnet`

- URL: https://ebible.org/Scriptures/engnet_usfm.zip
- Downloaded: 2026-11-07. Upstream source files dated 2026-10-30 (see `copr.htm`).
- Zip sha256: `f6541e3790f9fc47ec77cb6cb574e85a0555db1c19ed1e7f588724a0eae606c1`
- Copyright © 1996-2016 (2019) Biblical Studies Press, L.L.C. Not public domain. Terms: https://netbible.com/copyright/
  (reviewed by the user 2026-11-07): in a non-commercial work the text **without the NET Bible notes** may be quoted
  in any form without written permission, with the acknowledgement (`NET.rights` in text-build) and "NET" shown
  with it, hyperlinked to http://netbible.org in software with internet access.
- Contents: 66 books, USFM with Strong's numbers. Vendored verbatim (only the `.usfm` files and `copr.htm`).
- Notes: this export does **not** carry the NET's ~58,500 translators' notes. It has only Isaiah 43's (22, which
  text-build finds and leaves out) and `[[EMPTY]]` placeholders where a verse is left out. eBible's epub
  (https://ebible.org/epub/engnet.epub) has the same. Placing the full notes later needs another source.
- Versification: the KJV's, except that the 16 verses the BSB also leaves out are `[[EMPTY]]` placeholders (empty
  slots), and 2 Corinthians ends at 13:13 (the KJV's 13:14; eBible's empty 13:14 placeholder is no slot).
- Known export defects, handled in text-build rather than here: spaces inside punctuation (`happened )`,
  `‘ Man`), and the heading before Ecclesiastes 2:1 running on into the verse (a `Fix` in `NET.fixes`).

This directory is the exact input of `text-build` for `net-2026-v1`. **Do not edit or refresh it**: the
tokenization (and therefore every user's tagging ordinals) is derived from these bytes, and the golden checks in
`fixtures/text/` will fail. A newer NET means a new `text_id` (and a remap of taggings, if anyone has them).
