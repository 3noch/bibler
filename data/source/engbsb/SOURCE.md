# Source: Berean Standard Bible, eBible.org `engbsb`

- URL: https://ebible.org/Scriptures/engbsb_usfm.zip
- Downloaded: 2026-10-23. Upstream source files dated 2026-10-20 (see `copr.htm`).
- Zip sha256: `ce8bb27e62ab433e9eae460b65bf10710fabfd74017a9535aa8904343866e257`
- License: Public Domain (dedicated by BSB Publishing, LLC; see `copr.htm`).
- Contents: 66 books, USFM with Strong's numbers. Vendored verbatim (only the `.usfm` files and `copr.htm`).
- Versification: the KJV's, except 16 verses the BSB leaves out (in footnotes): Matthew 17:21, 18:11, 23:14;
  Mark 7:16, 9:44, 9:46, 11:26, 15:28; Luke 17:36, 23:17; John 5:4; Acts 8:37, 15:34, 24:7, 28:29; Romans 16:24.
  They are empty verse slots, so both texts have the same slots and verse addresses.

This directory is the exact input of `text-build` for `bsb-2026-v1`. **Do not edit or refresh it**: the
tokenization (and therefore every user's tagging ordinals) is derived from these bytes, and the golden checks in
`fixtures/text/` will fail. A newer BSB means a new `text_id` (and a remap of taggings, if anyone has them).
