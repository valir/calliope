# Calliope UI guide

<!-- Owned by Valentin. This is direction, not design work: describe how Calliope should look
     and feel, and the agents turn it into a consistent UI. Lines marked (draft) are
     suggestions to keep, change or delete. Sketches (ASCII, or photos of paper drawings)
     go in docs/ui/ and are linked from here; the agents can read images. -->

## Feel
<!-- A few words or sentences: the mood, and what matters most. -->
- Calm and focused: a tool you glance at while holding a guitar, not a dashboard.
- Readable from a distance: the laptop may be on a stand 1-2 m away.

## Usage situation
<!-- Where and how the app is used. This drives sizes, contrast and controls. -->
- Practice at home and possibly on stage; often a dim room.
- Hands are on the guitar while playing: during playback, the important actions must
  work from the keyboard, never only with small mouse targets.
- The Roland FC-300 controls only the gear, never Calliope. Calliope will later send MIDI to
  the FC-300 and the gear, not the other way round.

## Theme
- Dark by default, with a light theme available in Settings.
- Accent colour: amber
- Fonts: component default font

## Layout and navigation
<!-- Main structure. The views come from specs/overview.md; adjust freely. -->
- Navigation on the left (collapsible), with the content area on the right and a
  slim footer (status, edge-AI server state, version).
- Views: Library · Import · Editor · Playlists · Player · Export · Settings
- In the Player, the tablature takes most of the screen, with transport controls
  (play/pause, position, loop, tempo) in a bar that stays visible beneath the
  tablature

```
(draft sketch)
┌──────────┬──────────────────────────────────────────────┐
│ Library  │                                              │
│ Import   │              current view                    │
│ Editor   │                                              │
│ Player   │                                              │
│ Playlists│                                              │
│ Settings │                                              │
├──────────┴──────────────────────────────────────────────┤
│ status · edge-AI: connected · v26.10.0042               │
└─────────────────────────────────────────────────────────┘
```

## Keyboard
<!-- Shortcuts that matter to you. Defaults below can be changed. -->
- `Alt+1` … `Alt+7`: switch views in navigation order
- `Space`: play/pause in the Player
- `b`: mark section start
- `e`: mark section end
- `←` / `→`: previous/next section · `L`: loop the current section
- `[` / `]`: tempo −/+ 5%

## Window
- Minimum size: 1024×640. It should also look right maximised on a 1920×1200 screen.

## References
<!-- Apps or screenshots whose look you like (or dislike), and what about them. -->
-

## Sketches
<!-- Links to docs/ui/*.png|jpg, or ASCII sketches per view. -->
-

## Don'ts
<!-- Things you never want to see. -->
- No pop-ups interrupting playback.
- No tiny controls for anything used during playing.

## Decided by the team
<!-- The agents record here UI choices they made where this guide was silent, so the next
     feature reuses them. Edit or overrule anything. -->
- (gui-frontend-foundation, **owner decision**; amended by gui-export) Navigation order and `Alt+1`...`Alt+7`:
  Library, Import, Editor, Playlists, Player, Export, Settings (Alt+4 = Playlists, Alt+5 = Player,
  Alt+6 = Export, Alt+7 = Settings).
  Note: the ASCII sketch under "Layout and navigation" still shows the old order (Player
  before Playlists); the owner will update it.
- Font: Inter Variable, bundled with the app (shadcn-svelte itself sets no font; the system
  default differs per machine). Root font size 18 px, so everything scales up for reading
  at 1-2 m; nav items and buttons are at least ~50 px tall.
- Colours: shadcn "neutral" palette for both themes; accent/primary and focus ring are
  amber-500, with dark text on amber (white on amber is too low-contrast).
- (gui-tracks-repository, task 16) Disabled buttons: muted fill and muted text (primary and
  outline), plus 50% opacity. Selected tree/tablature rows: amber tint with an amber left
  edge. View-mode fields are borderless, edit-mode fields are boxed. Edit mode dims the
  toolbar and tree to 50% and shows "Finish or cancel editing to browse".
- Focus: a 3 px amber focus ring on every focusable element when using the keyboard.
- Navigation: buttons with an icon, label and shortcut hint; the active view has an amber
  bar and a highlighted background. `Ctrl+B` collapses the nav to icons only (not
  remembered between runs).
- Shortcuts switch the view and move the focus to the view's heading; mouse clicks leave
  the focus on the nav.
- Footer: "Ready" · "Edge-AI: not configured" (placeholders for now) on the left, and on the
  right a `v<version>` button that opens the About dialog (name, version, copyright).
- Settings: "Appearance" (theme: Dark / Light) comes first, then MIDI interface, Audio output
  and Edge-AI server.
- Placeholder views show the title, a one-line summary and "This view will be filled by:
  <feature>". Editor shows tabs (Stems, Tablature, MIDI cues).
- Window: 1280x800 centred on first start; afterwards the last size and position.
- Components: shadcn-svelte button, input, label, dialog, tabs, radio-group, separator, card.
  No toast library (sonner), which also fits "no pop-ups interrupting playback".
- (gui-tracks-repository) Library: two columns. On the left (20 rem) are a toolbar (Collapse
  all, Expand all, search box, Clear search) and a band > album > track tree, all collapsed
  at first and sorted case-insensitively, with "(no band)"/"(no album)" last. On the right is
  the track pane: labelled fields (this area scrolls), and below it, pinned, the "Tablature
  Files" panel (a list 4 rows high, plus Add · Update · Export · Remove) and the action bar
  Edit · Save · Cancel · Export · Delete, so every button is visible without scrolling at
  1280x800. A save conflict keeps edit mode and the draft, shows "The track changed on disk.
  Cancel to reload it; your edits will be discarded."; Cancel/Escape then reloads at once. Read-only fields look like plain text in disabled inputs;
  edit mode enables the inputs and locks the tree.
- Confirmations use a centred Yes/No dialog with the focus on "No". Results and errors are shown
  inline in the pane (status/alert line), never as toasts.
- Library keys: Ctrl+F search, ↓ from search into the tree, arrows in the tree, Ctrl+E edit,
  Ctrl+S save, Escape cancels editing.
- (gui-library-album-order) Tracks sort by album position, then title; albums by position, then
  name, with "(no album)" last. The position is shown as "3. " before the title, and albums show no
  number. Reordering uses pointer drag and drop in the Library tree only (the dragged row at 50 %,
  a 2 px primary line marks the drop gap, Escape cancels, the column scrolls near its edges), or
  Alt+Up / Alt+Down. The toolbar's Delete is an icon button after Expand all (tooltip "Delete the
  focused track or album (Delete)"). Renames are confirmed with old name, new name and track count.
  The track pane shows Position read-only after Type.
- Settings order: Appearance, Track repository, MIDI interface, Audio output, Edge-AI server.
- (gui-stem-extraction, task 22) Message text in the accent colour (import errors, edge-AI/tool
  problems, the "Change in Settings" link) uses `text-amber-700` in the light theme and
  `text-primary` (amber-500) in the dark theme, because amber-500 on white is only about 2:1.
  Buttons and badges keep amber-500 fill with dark text in both themes. A failed extraction
  scrolls its message into view (it sits below the fold at 1024x640). The switch's off state has a
  muted border so it stays visible on dark cards.
- (gui-import-local-batch) Import source page: four radios (URL, Local Audio File, Local
  Video File, Local Directory; the new one last so the keyboard order stays). A directory
  import shows `File N of M: name`, the current file's progress, a scrolling list of the files
  done and one Cancel (the only focusable control); the summary lists the saved tracks (with
  dropped stems) and the failed files (accent colour), then Show in Library (the first saved
  track) and Import another track. A cancelled batch returns to the source page with a status
  line.
- (gui-backing-track-editor, task 19) Editor layout. Each stem lane is one row (name, Play,
  Unmute, volume slider that takes the remaining width, dB value) when the pane is wide enough
  (1280x800) and two rows (slider and dB value below) at 1024x640, where the pane is only about
  380 px wide. The Mix lane is always three rows: Mix cell, Play/Pause, Stop; position slider
  and time field; Save and the target file. Lanes scroll; the Mix lane and Save never leave the
  window. Nothing moves when state changes: the solo name and the CLIP badge live in a fixed
  cell beside "Mix", the save progress replaces the "Save as" text instead of adding a row, and
  errors and the save result share one reserved two-line slot. Solo is shown by the amber
  "Solo" button, dimmed other lanes and the "Solo: X" text. The accent text in the Mix lane uses
  `text-amber-700` in the light theme (contrast). The 0 dB tick is a 2 px line on the volume
  slider. Control sizes: buttons 3.25 rem high, Play 4.5 rem wide.
- Known limit: at 1024x640 only about 1.5 stem lanes are visible above the Mix lane, so six
  stems need scrolling. Collapsing the navigation (Ctrl+B) gives the lanes more width.
- (stems-beat-autodetect, task 20) Editor Mix lane. The Mix lane starts with the shared waveform
  (3.5 rem high, same component as the Player), then Mix cell, Play/Pause, Stop and "Autodetect BPM"
  (becomes Cancel while detecting; disabled with its reason in the message slot when there is no
  edge-AI server, the server has no beat tracker or nothing is unmuted); position slider and time;
  Save and its target; the two-line slot (phase text such as "Detecting beats", the result
  or the error). The lane was made tighter (padding, gaps, 3.5 rem waveform) so that four stem lanes
  and the whole Mix lane fit at 1280x800. When the window is too small (1024x640), the Mix lane wraps
  (Autodetect below Play) and the whole Editor pane scrolls; the lane list keeps at least 14 rem.
- Tempo sections in a waveform (Editor and Player): an amber 2 px bar at each section start and a label
  "<bpm> BPM" (bold 15 px, readable at 1-2 m) right of it, or just the number when the next bar is close.
  In the Editor, changes typed in the Player are dimmed (read-only) and detected ones can be moved: drag the bar (8 px
  grab zone, the pointer shows a resize cursor, a readout at the top left gives "+N/64 beat (time) · limit 4 beats") or focus the waveform and use Left/Right (1/64 beat), Shift+Left/Right (1 beat),
  Up/Down (select the change; the selected one is a 4 px bar). The waveform is locked during a
  detection. A detected change sits beside the
  loop marks without colliding (loops have a cap on the bar, sections do not).
- (gui-player-tempo-sections) Player sections are editable. Every change can be dragged (no read-only
  dimming in the Player); the waveform in the Player selects a section by a click (4 px bar, and the label
  underlined), and the selected section, else the one at the playhead, is the target of the Sections group.
  A selection holds only until the playhead enters another section (playback crossing a boundary, a seek or a
  jump there); from then on the section under the playhead is the target.
  A change moves in 1/64-beat steps, at most 4 beats from where it was detected or added; edits made
  while playing never stop the audio, and the clicks follow from the next beat. Keys with the waveform
  focused: Up/Down select a section, Left/Right move the change by 1/64 beat, Shift+Left/Right by a beat,
  Enter edits the BPM, Delete or Backspace removes the change, `a` adds one at the playhead; Space,
  `b`, `e`, `l` and Shift+L keep their Player meaning there. Clicking a label opens the BPM overlay: a
  small field over the label (for Add: at the playhead), prefilled and selected. Enter commits (20 to
  300; otherwise "Enter a BPM between 20 and 300" shows in the group's message line and the field stays),
  Escape or clicking away cancels, so nothing is added by accident.
- The Player's control bar (gui-player-tempo-sections, phase 3) has two rows. Row 1 holds Sections and Loops side by
  side, row 2 is Playback at the full width (Rew, Play/Pause, FF, the time box and the position slider), at the
  bottom where Play is easy to find. There is no master metronome, no global Count In and no Detect button; the
  disabled -/+/Reset BPM placeholders are hidden until that feature exists.
- Sections group (4 rows). Title row: "Sections", the BPM input with its V box (the section's BPM; the
  target section is the one selected on the waveform, else the one at the playhead) and the name box (10 rem
  wide, "Name this section"). Button row: eight 2.75 rem buttons, Add (+), Remove (-), x2, /2, Prev, "|<" (jump to the
  section start), ">|" (jump to its end: the last frame before the next section) and Next; each disabled one has a title that says why.
  Third row: Click
  ("Metronome in this section"), Count In ("Count-in into this section") and the status ("Section 2 of 3 · 93 BPM", or
  "No tempo"). Fourth row: "Start" [offset field] "/64 beat" the offset's time, then the "Find start" button. The message line is an ordinary `min-h-5` span below (single line, truncated with the full text as tooltip);
  the BPM note can change the height of the BPM group.
- Section toggles: Click and Count In are filled amber when on. A new tempo map starts with Click on in every
  section and Count In off. Count In is enabled for every section, the first included (a count-in before the
  first section is the six count-in ticks at its tempo before the song starts); both are disabled when there is no BPM ("Type a
  BPM first"). Which beats click is decided by the section flags alone.
- Play count-in: Play from paused or stopped counts in (six ticks over 5 beats at the tempo of the section under the
  playhead, the 5th beat silent, then the audio; the same pattern as the export count-in) when that section has Count In on. The clicks are on the song's beat grid at the
  heard speed: from a start between two beats the audio starts that much before the end of the 5th beat, so the next beat
  falls on the 6th beat position (the same for a loop starting between beats); elsewhere Play starts at once. A loop's Play counts in only
  with the loop's own Count-In. Older files are migrated: a track whose old Metronome was Off is silent (all section
  Click and Count In off), one whose old Count In was on gets Count In on its first section.
- Loops group (3 short lines, about 430 px): the "Loops" title, Prev, `+`, the loop box, `-`, Next (4 px gaps so
  the line fits); `>`, bar1, the progress slider, bar2, `<`; then Play/Stop, Loop On and Count-In. `+` at the playhead makes a complete loop
  at once: it ends at the next section change, or at the track end in the last section (or when nothing follows),
  so Play works straight away; `<` at or before the loop start is refused with a message. The third line also holds
  the training button (below); during a speed-training run the status line of the pane shows "Training 70 % · 2/4"
  (speed and repeat of the pass, "· 3" without a total at the end speed).
- Speed group (gui-player-speed), the Playback pane's second line: the label "Speed", `-` ("Slower") and `+`
  ("Faster") buttons (3.5 rem wide, 5 % steps through 25, 30 ... 200), the percent box ("Speed in percent",
  4 rem), "%", Reset (back to 100 %, disabled at 100 %) and the readout of the BPM being played ("120 BPM"; the
  tempo of the section under the playhead times the speed; empty without a tempo). The box takes a whole number from 25
  to 200: Enter commits, Escape or leaving with a bad value drops the edit; a bad value (20, 250, text) is refused,
  the box gets a red border and the readout place shows "25-200 %" (nothing moves). The keys `[` and `]` step
  slower and faster (ignored while a form control has the focus, like the other Player keys). All of it is disabled
  with a title that says why while a speed-training run is active (stop the loop first); `[` and `]` are refused then too. The pitch stays the same.
  Narrow panes (1024x640 with the tree shown, 412 px) wrap the group onto a second line inside the pane rather than
  clip it.
- Training button and popover (gui-player-speed): the third Loops line ends with a button whose visible label is
  "Train..." (the plan's "Training..." wrapped at the button's width; its accessible name is "Speed training
  settings"). It is disabled until the current loop is complete (title: "Set the loop end first") and filled amber
  while the loop's Training is on. It opens a non-modal popover (role "dialog", "Speed training": the Training on
  switch, Start %, End %, Step %, Repeats, a summary line, a message line and Close), 20 rem wide, to the left of
  the button (it was planned above the button, but at 1024x640 with the tree folded the 418 px popover fits neither
  above nor below the button; to the left, floating-ui only has to shift it up or down). The focus goes to the switch
  on opening; Tab order: switch, Start, End, Step, Repeats, Close, and Tab from Close closes it. A box commits on
  Enter, Tab or leaving it (a bad value shows its message and keeps the text); Escape closes the whole popover and
  drops an uncommitted edit, so does a click outside; closing by Escape, Close or Tab puts the focus back on the button.
  The frontend logs `player training-popover open ... rect=... viewport=...` (after it is placed) and
  `close via=<escape|outside|button|tab>`.
- Name box: text up to 40 characters, trimmed. Enter or leaving the box saves, Escape drops the edit. While it
  has the focus it keeps the section it was focused for (the playhead moving does not change it). An empty
  name removes the name. The Player's letter keys and Space type normally in it.
- Start / End (and the Home / End keys, which also work with the focus on the waveform or the page): jump to
  the section's first beat or to its last frame (still inside the section, so pressing End again
  stays); while playing, playback continues without a gap.
- Layout, measured by the `player layout` log (overflow = scrollHeight - clientHeight of the pane): at 1280x800 with
  the tree folded the pane is 965 px wide, Sections (496 px) and Loops (424 px) share row 1, Playback is the bottom
  row (two lines since the Speed group), and nothing scrolls (overflow 0; target at most 14 px; log
  `overflow=0 width=965 loops_beside=1 sections_w=496 loops_w=424`). Measured positions there (window coordinates):
  Sections/Loops top edge y 307, Loops lines y 350 / 418 / 485 (Play/Stop x 889, Loop On 981, Count-In 1090,
  Train... 1194; with the Start row of Sections, task gui-first-beat-tuning; Sections rows y 341 / 394 / 458 / 511, buttons
  `+ - x2 /2 Prev |< >| Next` at x 373 / 432 / 490 / 548 / 607 / 666 / 724 / 782, Start field x 432, Find start x 694),
  Rew/Play/FF row y 628, Speed row y 695 (`-` x 451, box 528, `+` 633, Reset 713, readout from 763).
  With the tree shown (668 px wide) Loops wraps below Sections and the pane scrolls by about 234 px; at 1024x640 it
  scrolls by about 349 px (tree folded) and 879 px (tree shown, 412 px wide, where the Sections row is clipped on
  the right: the name box and the last button are cut). The groups never overflow sideways except in that last case.
  The training popover at 1280x800 tree folded is at x 782-1142, y 86-515; at 1024x640 (tree folded or shown, the pane
  scrolled to its end) at x 283-636, y 15-434: inside the window in all four size/tree cases.
- (gui-play-backing-track, task 17) Player layout. Left the shared track tree (20 rem, Ctrl+L folds it to
  a rail), right the pane: source row ("Play:" dropdown 15 rem wide, then the title), the dashed
  tablature placeholder (it gives way first), the waveform (5 rem), then the control bar described above.
  Nothing moves when state changes (every message has a reserved line).
- Toggle buttons that are on (section Click and Count In, Loop On, loop Count-In) are filled amber
  with dark text in both themes. The `bg-amber-500!` form is needed because the outline button variant
  sets its own dark-theme fill. Waveform marks use `--primary` (amber); the current loop is solid, the
  others 55% opacity, a loop without an end is a start line only; the playhead is a 1 px foreground line.
- Player keys: Space, b, e, Left, Right, l / Shift+L, Home / End, `[` / `]` (speed), Ctrl+L, Ctrl+F; they are ignored while a form
  control or the select has the focus, so Space still presses a focused button. The waveform is the
  exception: focused, it keeps Space, b, e, l and Shift+L, and uses the section keys above (Left and
  Right move the selected change there, not the loop).
- Known limits: at 1024x640 the controls need scrolling (about 1.5 screens high with the tree
  shown, and the Sections row is clipped there), and the unfilled part of the position slider is faint in the light theme (the slider is
  shared with the Editor).
- (gui-export) Export view. Left the shared track tree (read-only, the same selection as the other views),
  right the pane: Band, Album and Title as read-only rows, the source line and the count-in line, a
  bordered group "Export..." with three 3.25 rem buttons ("to BOSS RC-5" with its "Memory" box beside it,
  "MP3 file", "OGG file") and a muted reason line for a disabled state, then the job area (reserved
  height): while running a progress bar with Cancel (the focus moves to Cancel) and "Exporting <title> to
  <path> - 42 %"; afterwards "Written to <path>." (RC-5: plus the eject sentence), or, in the accent
  colour, "Export failed: <message>"; "Export cancelled; nothing was written." Buttons are disabled for a
  band or album row, a track without a usable backing or tempo map, and while an export runs. Overwriting
  asks first (Replace the existing file ...? focus on No). Measured: no vertical overflow at 1280x800; at
  1024x640 (tree shown) the Memory box wraps below its button and the count-in line takes two lines.
- (gui-export-info) Export info. The Library track pane (read-only, never editable) gets one extra
  row after Modified for each kind that was exported, and none for a kind that was not: "RC-5"
  (`memory 7 (2026-10-24 18:30)`), "MP3" and "OGG" (`<path> (2026-10-24 18:30)`); the time is local
  time, `YYYY-MM-DD HH:MM`. The Export view shows a muted "Last: memory 7, 2026-10-24" line (RC-5,
  MP3 and OGG: each under its button) only for a kind that was exported, with the
  path breaking anywhere so a long one wraps inside the pane. When the export succeeded but the
  record could not be saved, an accent-colour line under the result tells why (the file is written
  anyway). The Memory box follows the selected track (its recorded RC-5 memory, else the
  last one used plus one); a refill after an export shows the memory just used. Measured: at 1280x800 the Export view
  has no overflow with all three "Last:" lines. Each "Last:" line has a reserved height under its
  button (one line for RC-5, two for MP3 and OGG), so the buttons sit at fixed y positions whatever the
  text (a path of two lines or fewer moves nothing; a longer one pushes the rows below it down); at 1024x640 the Memory box sits under its button and the
  pane scrolls. In the Library the rows are at the end of the scrolling field list.
- (gui-first-beat) Player waveform: the part before the first beat is dimmed (the background colour at 60 % over
  it). The first bar is drawn like the other bars, also at 0:00, with its BPM label to its right; it is dragged
  and moved by keys like the others, and its readout reads "+N/64 beat (time) from the found start". The "Find
  start" button sits at the end of the Sections group's fourth row (the Start row, a smaller text, so that Sections and
  Loops stay side by side at 1280x800) and is disabled outside the first section ("Find start works in the first
  section"); its message line
  says "Finding the start..." while it runs and then "First beat at 0:01.200" (the time found); errors use the
  destructive text colour.
- (gui-first-beat-tuning) Keyboard readout: a key on the focused waveform (Left / Right, with Shift) shows the same
  readout as a drag ("+N/64 beat (time) from the found start", top left of the waveform; in the Editor's Mix waveform
  too) and keeps it 1.5 s after the last key; it is gone at once on blur. The "Start" field (Sections, row 4) takes the
  offset of the section under the playhead in 1/64 beats, "+10", "-8" or "10": Enter applies it, clamped like a drag (the
  field then shows the value really applied), Escape drops the edit, and leaving the field applies it; text that is not
  a whole number is refused with "Type a whole number of 1/64 beats, like +10 or -8" in the message line (destructive
  colour) and nothing moves. Prev / Next go to the start of the previous / next section and behave like "|<" while
  playing (playback goes on, no count-in); Prev is disabled in the first section ("This is the first section") and Next
  in the last ("This is the last section").
