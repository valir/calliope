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
- Views: Library · Import · Track · Playlists · Player · Settings
-  In the Player, the tablature takes most of the screen, with transport controls
  (play/pause, position, loop, tempo) in a bar that stays visible beneath the
  tablature

```
(draft sketch)
┌──────────┬──────────────────────────────────────────────┐
│ Library  │                                              │
│ Import   │              current view                    │
│ Track    │                                              │
│ Player   │                                              │
│ Playlists│                                              │
│ Settings │                                              │
├──────────┴──────────────────────────────────────────────┤
│ status · edge-AI: connected · v26.10.0042               │
└─────────────────────────────────────────────────────────┘
```

## Keyboard
<!-- Shortcuts that matter to you. Defaults below can be changed. -->
- `Alt+1` … `Alt+6`: switch views in navigation order
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
