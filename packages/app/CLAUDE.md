# packages/app: Solid client

(Moved from the root CLAUDE.md.)

## UI layout (redesigned 2026-10-11; the user found the first version wordy and clunky)
- Text is the whole screen: `Bible.tsx` = continuous scroll over chapters (window of <= 9 rendered, extended at
  either edge within 1500px, manual scroll anchoring so edge changes never move the visible text; books are loaded
  BEFORE a chapter is inserted so nothing renders empty). The URL hash follows the chapter under the top bar
  (`history.replaceState`); typing a ref or `hashchange` navigates. Painting/selection work across chapter and book
  boundaries (one hit-testing container); across books they become one tagging per book (`index.byBook`), as one undo
  step (`store.paint/erase/eraseAll` take a list of intervals).
- Top bar (must fit a 320px phone): menu button, location field (shows "Psalm 23"; click/`/` to type a ref; at rest
  it is styled as a field with a magnifier, outline and `/` hint because a bare label didn't read as editable; the
  translation's abbreviation sits at its right end only when the full chapter name fits beside it: the chapter wins), sync
  dot (text only when offline), and the open group's name (only in a group; opens the spaces card). No permanent
  side panel, no help text.
- **Pages** (2026-11-01; `Bible.tsx` `paged`): a chapter at a time, or the part of one a licensed text's limits allow
  (`chunksOf`/`pageSlots` in packages/scripture limits.ts), with the verse before and after dimmed (`.chapter.context`,
  no `data-ch`, skipped by `topVerse`) so tags across the boundary show and can be painted; ‹ › at the top and bottom
  name the neighbouring pages and are reading on (not jumps); a jump lands on the page holding its target, which
  opens with its top band out of view (`.page-body` is at least a screen tall). **Run-outs** (2026-11-01, replacing a
  pull-past-the-end gesture the user tried: the browser's pull-to-refresh took the pull down, and the rubber band kept
  the bar from filling): a band past each end ("Keep scrolling for John 4", a bar filling as it scrolls into view);
  scrolling to its end turns the page, keeping the verse read next (this page's dimmed neighbour, the next page's
  own first) at the same place on screen, so the new text appears where the band was and the eyes needn't move (the
  user's wish; a chapter heading the new page has before it appears above it). A **seam mark** (2026-11-02, the user
  still found the turn jarring) marks that verse: a 3 px accent bar in the left margin that fades in with the band,
  stays through the turn and fades out 1.5 s after (`MARK_HOLD_MS`). Ordinary scrolling, no overscroll. Only the user's own scrolling toward a band turns pages
  (`pinned` false, and down for the bottom one, up for the top one: scrolling back out of a fully shown band once
  turned the page), so a short page showing its band at once doesn't turn by itself. The turn eases in (the user
  found it jarring): the shared verse dims from full strength over 0.5 s, the rest fades in over 0.35 s in place (no
  movement, which would undo the eyes staying put); other page changes fade the page in over 0.2 s; none with
  `prefers-reduced-motion`. A reading setting under Aa ("Scroll" / "Chapter by chapter", per device) and always on for a
  limited text, whose pages come from `/api/text/<id>/page` (`loadPage`, never cached), whose license line ends each
  page, and whose snippets in lists show only the place ("Tap to read it on its page"): showing many places' words
  at once would break its limits. The menu marks each translation "Can be read offline" or "Online only, a page at a
  time"; limited texts aren't kept offline and aren't offered in the offline-only build. Browser-checked (BSB-R John 3
  and Jude's three parts, only manifest/index/page requests, the KJV switched both ways, run-outs down and up with
  the shared verse kept in place within a few px). Not yet: a real trackpad or phone.
- **Back** without browser buttons (2026-10-31, the user's design; an installed app has none): for 6 s after each step
  (a jump of the user's own, or Back/Forward; not the recent places a fresh launch adds, `trail.i > trail.seeded`; not
  after a reload) a small bubble under the search bar, "← John 3", goes back (with the panel that was open there); it
  then goes away entirely (the user found a lingering arrow annoying). After that, the search bar's empty-box list
  offers "← Back to John 3" first (whenever Back leads anywhere, seeded steps too), and the menu has Back/Forward.
- Menu (`Menu.tsx`, a left drawer; decided 2026-10-14 because the top bar overflowed on phones): its header holds
  what is seldom needed, as icons (2026-10-31: the user wants the list for what people need most of the time): Close,
  then **Back/Forward** (2026-10-31: this tab's jumps, like the browser's buttons, which an installed app lacks; greyed
  out when there's no step, labelled "Back to Romans 8"/"Forward to John 3:16"; App keeps `trail` = each step's entry
  plus the current index, which every step's state also carries as `{entry, i}`, in sessionStorage, since the browser
  says neither whether a step exists nor where it goes; links followed in an open tab and a fresh launch's seeded steps
  are steps too; each step also remembers the panel that was open when it was left (`trail.ui`: Nearby, a tag's
  sheet, the tag list, search chips), and Back/Forward reopen it with the place), **Aa** (opens the text's font and size under the header) and Help. Below: account (sign in,
  or who you are + sign out; only when the server offers sign-in), space row (opens the spaces card),
  translation (collapsed; the texts on offer with their rights lines, and a note that tags show in the
  translation they were made in), recent reading (collapsed; see below), contents. **Text font and size**
  (2026-10-31, `reading.ts`: font Serif/Book/Sans, all system stacks
  so nothing downloads; size 80-190% in steps, the percentage resets to 100%; per device in localStorage
  `bible-tags/reading/v1`, decided with the user: not per account, group or translation, since the right size
  depends on the screen; applied as `--read-font`/`--read-scale` on the root, used only by the reader's styles, and
  the page width scales with the size so lines keep their length; a change keeps the verse at the top, also across
  quick repeated clicks). Recent reading lists the reading history's entries, newest first, as "Genesis 12:3–15:4" + how long
  ago; choosing one goes back to where it left off. Contents: Old/New Testament books; a book opens its chapter grid, the current one starts open; one-chapter
  books go straight there; choosing closes the menu. At the very bottom (seen only by scrolling there) a faded version stamp (`Version.tsx`: the app's commit from index.html's meta tags and the server's from the last sync `ready`).
- **Undo/redo** (2026-10-31, the user's design; they had looked like Back/Forward in the menu header): in the dock after
  the eraser on wide screens; at <= 560px (where well names hide) in the All-tags sheet's header beside Close, since a
  phone's dock fills up with tags. Hidden for view-only members. Every undo or redo (buttons, ⌘Z, ⇧⌘Z) shows a bubble
  for 4 s saying what it did ("Undid tagging John 3:16 with “Love”", "Redid creating the tag “Love”", "Undid adding a
  comment on Romans 8:28"); after an undo it carries Redo (on a phone the quick way to redo), and an × closes it early. Wording:
  `describeChanges` (packages/scripture `describe.ts`, tested) from the step's changes (`SyncState.stepChanges`,
  read before reverting; a redo step's changes are reverts, read inverted); a paint that extends a tagging names only
  the new words; a place in another translation is left out. Considered and deferred: an undo pill after each paint.
- Dock (bottom pill) = palette: tag wells (names hidden under 560px except the active one), `+`, eraser. The
  active well shows a **brush** (not ✕: the user read ✕ as "delete") and tapping it again stops painting (no
  separate Done button; Esc also stops). The active well also carries a visible **pencil** = edit sheet (the user
  found right-click too subtle; right-click / touch long-press remain as shortcuts).
- **Unhighlight** (the primary way to remove ONE tag from an overlap; the eraser removes everything): every
  "Here" card in Nearby has a labelled Remove button, and every snippet of a single tag's instances in the tag
  sheet and single-tag search has an eraser button that erases exactly that tagging (`store.erase(tag, start, end)`; undoable). Combination rows get none (a
  combination isn't one tagging). Visible on the current row and on hover; always on touch devices.
- Selection rules (`Main` in `App.tsx`): the eraser and a selected tag are exclusive (eraser on deselects the tag
  and erases ALL tags; picking a tag turns the eraser off; Alt-drag with a tag erases just that tag). All selection
  goes through `selectTag`/`toggleEraser`. An effect deselects the active tag whenever it stops existing (deleted
  here or elsewhere, creation undone). Gotcha: deleting from the tag sheet unmounts it, so read ids before calling
  the store, never after (a stale `<Show>` accessor silently broke this once).
- **Palette reorder**: drag wells in the dock (mouse: press + move > 6px; touch: long-press 350ms then move; a plain
  swipe scrolls the dock; long-press released without moving = edit). Gesture logic lives in `Dock` with pointer
  capture on the `.wells` container (wells move in the DOM during the drag); the live order is a preview, committed
  as ONE `palette_set` on release (so undo restores it; number keys follow the order). The click after a drag is
  suppressed. Keyboard: Alt+←/→ on a focused well.
- **All tags** (`Tags.tsx`): dock button (always shown; badge = tags not in the palette) opens a list: a
  "Find or create a tag…" box (focused; an unmatched name shows "+ Create “name”", Enter creates and starts
  painting with it; Enter on an exact match picks it), places count,
  pin/unpin (palette max 9; pin disabled with a tooltip when full), pencil = edit. Tapping a row paints with that
  tag. An active tag that is not in the palette shows as an extra well at the end of the dock (so a tag created when
  the palette is full is visible); the active well scrolls into view. Panel priority: Nearby > tag editor >
  All tags > search results (closing the editor returns to the list).
- Tag sheet: name shown as text + pencil (click to rename; Enter/blur saves, Esc cancels), color circle with a
  pencil badge over a hidden color input; instances use the shared `ResultList` (snippets by book, stepper, n/p).
- Location label = the chapter at the middle of the reading area; after a navigation it is pinned to the target's
  chapter until the user scrolls by hand (wheel/touch/scroll keys/pointer), else a centered target or a very short
  chapter would name a neighbouring chapter.
- Tapping tagged words opens the **Nearby** panel (`Nearby.tsx`; right drawer, bottom sheet on phones). First
  **Here** (2026-10-14; the user found removing/commenting hard when the place sat mid-list between its neighbours):
  one card per tag with its tagging covering the position (the clicked word, or the instance opened since): Remove,
  edit, the snippet, and its comment thread always open. Then, for navigation only (no erase buttons or threads), the
  combination of all tags at that word first (`intersectRuns`), then each tag alone, each a window of nearby
  instances (`nearby`, +-5, "N earlier/later" to expand; a list whose only instance is the one here is left out,
  2026-10-19: the user found a tag used once shown twice) shown as text snippets: <= 40 words with up to 10 words of
  dimmed context clipped to the instance's verse(s); longer ones show 40 words and fade. Clicking one navigates
  (centered + flashed) and re-centers the panel on it. Pairs of a 3+ tag set are NOT shown yet.
- **Location hash**: `#/<translation abbr>/<ref>` written by the app (`hashOf`); links may omit the translation
  (`splitHash`/`findText` in `src/text.ts`; App waits for texts.json only when a link names one).
- **Search** (`Search.tsx`; designed 2026-10-11): the top box is "go to or search". Typing suggests "Go to <ref>"
  (default on Enter when no chips, so plain navigation is unchanged), "<ref>: search within", matching tags
  (substring, prefix first) and book groups (`BOOK_GROUPS` in `packages/scripture/src/scopes.ts`: OT/NT, Law,
  History, Poetry, Prophets (major/minor), Gospels, Pauline/General epistles). Picks become **chips**: tag chips
  are ANDed (`intersectRuns`, superset semantics), scope chips ORed (`inScope` keeps instances overlapping any
  scope). Chips instead of a typed grammar because tag names may contain spaces. Scope-only query = "what's tagged
  here" (tags with counts; clicking adds the tag). Results panel = same sheet as Nearby: grouped by book, 50 per
  page, stepper ‹ i/N › and `n`/`p`; Nearby sections have "Search all". Backspace removes the last chip; Esc clears.
  Not yet: query in the URL, highlighting only matching instances in the text, "A&B and nothing else".
- **Offline-only build** (`VITE_SYNC=local`; GitHub Pages): `localServer.ts` stands in for the sync server inside
  the browser (same protocol and canonical `effect`), storing the confirmed state (rows + last 1000 changes = undo
  targets + the op ids of older changes, so a re-sent op is a duplicate forever) in localStorage
  `bible-tags/local-db/v1` under a Web Lock; ops are checked like the server's (`checkOp` in packages/scripture, the
  TS twin of `api::check`: ids, tagging ends against the text index, which it awaits); tabs exchange changes over a BroadcastChannel
  and a tab that falls behind gets a `reset` (a SyncState message added for this). `BASE_KEY` is not written in this
  mode. Text/asset paths use Vite's `BASE_URL` ("./" here) so any subpath works; routing is in the hash. The UI shows
  an amber "Local only" badge instead of the sync dot, and the help card explains that tags are kept only in this
  browser (not saved elsewhere, not backed up, not synced; clearing site data erases them). Browser-tested under a
  `/pages/` subpath: no `/api` requests, persistence across reloads, live sync between two tabs, per-tab undo.
  Limits: localStorage (~5 MB) holds everything; IndexedDB and export/import are the natural next steps.
- **Translator's notes** (2026-10-31; BSB footnotes 4,846, KJV marginal notes 6,959, built by text-build and
  anchored after a word): the † after the word is a button (an invisible tap area bigger than the glyph) that opens
  `NotePopover` (App.tsx): the verse, "BSB note", and the text with its references as links (`LinkedText`, shared with
  comments; 350 BSB notes say "Cited in …"). Below the marker or above if no room; a tap elsewhere, Esc or scrolling
  closes it; a link in it is a jump like a comment's. Pressing a marker never paints, even while painting. Replaced a
  hover title, which phones can't show. Browser-checked on both widths. Considered next (user agreed to start with
  this): notes in the tag panel for the tapped verse; later maybe notes as a read-only publisher "tag" in search.
  **Headings link their references** too (2026-10-31): every heading goes through `LinkedText` (inline), so the BSB's
  parallel-passage lines under gospel and history headings ("(Isaiah 40:1–5; Matthew 3:1–12; Mark 1:1–8; Luke
  3:1–20)", kind `r`) are links; all 1,324 link every passage (checked against the build). Ordinary titles have no
  chapter numbers and stay plain. A link pressed while painting doesn't paint.
- **Comments** (`Comments.tsx`): in Nearby, each "Here" card shows its thread; elsewhere a single tag's snippets
  (Nearby single-tag sections, tag sheet, single-tag search) show a speech-bubble count; the **current** one shows its thread (oldest first, "edited", hover pencil/trash, always
  visible on touch) and an "Add a comment…" box (never autofocused; Enter = newline, ⌘/Ctrl+Enter or Post posts; Esc
  cancels an edit). Combination rows have no comments. `store.commentsByTagging` follows merge chains. Browser-tested:
  post (focus stays), multi-line, edit, delete, undo of each, merge shows all comments on the survivor and undo splits
  them back, local-only build persists them. Not yet: a marker in the text itself, comments in the Nearby combo rows.
  **References in comments are links** (2026-10-31; rendering only, decided with the user: nothing stored, cross
  references and indexing may come later): `findRefs` (packages/scripture, tested and mutation-checked) finds them
  when a comment is shown, strictly: exact book names/abbreviations only (no typo forgiveness, no ambiguous ones like
  "Jon"), a chapter after the book, abbreviations capitalized, two-letter ones and lowercase names only with a verse
  ("Is 53:5", "john 3:16"; not "So 2 of them"), and a candidate the parser warns about is left plain; each item of a
  list is its own link ("Rom 8:28, 31"). `RefLinks` (Comments.tsx) gets the index, an `href` (so a link opens in a
  new tab like any) and `go` from App: a jump (so Back returns with the comment's panel open), after which the panel
  closes on a phone (<= 560px, where it covers the text) and stays on a wide screen. Browser-checked at both widths.
- **Keep list identity stable** (found while building comments): every edit rebuilds the view, so lists of fresh
  `[start, end]` arrays made Solid's `<For>` remount every snippet (a comment box lost focus mid-post). Use
  `intervalOf(tagging)` (one array per row object) for interval lists; `ResultList` groups with `<Index>`, keys
  snippets by interval, and resets its cursor only when the places actually change.
- **PWA** (installable, offline reading; both builds): `public/manifest.webmanifest` + icons (`icon.svg` drawn by
  hand, PNGs rasterized from it with `rsvg-convert`; the maskable/Apple ones are full-bleed with the art at 80%).
  `src/sw.js` is a template: the `serviceWorker()` plugin in `vite.config.ts` fills in this build's file list and
  a version hash and emits `sw.js`; `main.tsx` registers it in production only.
  Shell precached per version (old versions deleted); `text/texts.json` network-first into a `texts` cache; texts
  (2026-10-23, several translations) one cache each, `text-<id>`: the page sends `{type: "keep-texts", texts}` (the 3
  most recently read, `keepOffline` in `src/text.ts`, whenever one is shown; not in `activate`, where it stalled the
  page's requests on a first visit) and the worker downloads each whole, four files at a time, deletes files an
  older build of it left, and drops every other text's cache;
  hashed files cache-first; navigations and other files network-first with cache fallback; `/api/` untouched.
  Browser-tested under `/pages/`: 9 shell + 68 text files cached, and with the server stopped an unvisited book
  (Obadiah) opened. iOS gives an installed app storage separate from Safari (matters for local-only tags).
- **Hidden tags** (2026-10-12): per device and per space (localStorage `bible-tags/hidden/v1:<space|me>`),
  `store.visibleTaggings` feeds the text, tap-to-inspect and the eraser; search, Nearby lists and the tag list still
  show hidden tags. All-tags sheet: an eye per row plus "Hide all" / "Show all" under the search box; hidden wells
  in the dock are dimmed.
- **Spaces / groups** (`Groups.tsx`, `store.ts`; 2026-10-12): the menu's space row, or the top-bar chip (the open
  group's name and a "view only" badge), opens a card: My tags + your groups (click to switch: sets
  `bible-tags/space/v1` and switches in place, no reload (2026-10-16): the store saves and disconnects the old
  space's `Opened` (SyncState, caches, socket) and opens the new one; `store.space`/`store.me` are getters; the app
  stops painting and closes Nearby/tag sheet/tag list/search on a switch), "New group", and for the open group its members (admins: role select,
  remove), invite links per role (create / copy / new link) and Leave. Caches (base, outbox, undo) are keyed
  `<key>:<group id>` (the personal space keeps the old keys, so nothing migrates). `?join=<token>` joins before the
  store opens. View-only: `submit` refuses with a notice, and the dock's New tag/eraser, the tag list's
  create/pin/edit, the comment box and every way into the tag edit sheet (well pencil, right-click, long-press,
  Nearby pencils) are hidden, as are Remove/erasers on places; a sheet open when the role drops closes. In a group, comments show the author's name (member name, email or
  "Anonymous"; "You" for yours) and only your own have edit/delete. The role is refreshed on every socket open; a
  refused group socket re-checks membership and falls back to My tags. Not in the offline-only build.
- Touch screens don't autofocus the All-tags box (the keyboard covered the list): `matchMedia("(pointer: fine)")`.
- Testing UI on this machine: run a throwaway server on `127.0.0.1:<port>` (NOT `localhost`: cookies ignore ports,
  so a test server on localhost overwrites the user's dev session cookie).

## Client (Solid) notes
- Per-verse components, one `<span data-ord>` per word. Per-verse memo computes **segments**
  (runs of consecutive words with an identical tag set); only affected verses re-render on a change.
- Segment styling via CSS custom properties keyed by tag-set. Precedence: exact-match overlap rule > largest matching
  subset rule > default blend (stripes, or background + underline). Color must not be the only signal.
- Painting: event delegation + pointer events + `elementFromPoint` (touch works). During a drag only a transient
  stroke-preview signal changes; commit ONE tagging intent on pointer-up. Selecting nothing = native selection.
  Also: select text normally, then click a palette tag to apply.
- Palette: top-N pinned tags, reorderable, number keys switch tags, Esc = none.
- Overlap segments: a space between two runs that share a tag is tagged too (no white gaps inside a tagging).
- Spike later: CSS Custom Highlight API as an alternative to per-word spans (spans preferred for hit-testing,
  comment markers, overlap styles).
