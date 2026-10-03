# Design System: usage-flow

Character: the radio's station panel, turned into a gauge. Black, monospaced, sharp, lit by one light running from cyan to periwinkle. It sits open all day beside other work, so it stays still and quiet until a limit needs you.

Tokens come from email-flow's DESIGN.md, only the ones this panel uses. They live in `src/theme.rs`. No value outside this file without adding it here first.

## Color

- `--status` #000000: the panel
- `--surface-raised` #11202A: bar tracks, the plan badge
- `--surface-top` #172A36: tooltips
- `--border` #132631: the chart's baseline, the tooltip edge
- `--text` #E6FBFF: figures, the account name, the pace tick
- `--text-muted` #7FB2C2: limit names, activity, pace
- `--text-dim` #3F6674: labels, resets, timestamps, icons at rest
- `--accent` #00E1FF: the near end of the light, the active dot, the spinner
- `--info` #8C9EFF: the far end of the light
- `--warning` #FFB000: a limit from 70%, a pace that reaches 100% before the reset
- `--danger` #FF4D4D: a limit from 90%, errors

Account identity is a hue, shown as an 8px square before the name and never as text: #39FF9E, #EDE95C, #C792EA, #A6E22E, #F5A97F, #FF9CC2, #D8C8A0, #A9B8C2, in the order accounts are found.

## Typography

- Family: `JetBrains Mono` for everything, bundled. 400 regular, 600 for figures, the account name and uppercase labels. No 700.
- Scale: 11 / 12 / 15. Labels, resets and timestamps at 11. Limit names and the account name at 12. Figures (tokens, sessions, percentages) at 15, the places the eye should land.
- Case: lowercase everywhere. The one exception is the top row, 11px 600 uppercase with 1.6px tracking, as in the radio panel.

## Spacing

- Base unit 4px; scale 4 / 8 / 12.
- Panel padding 12. 4 within a group, 8 between groups, 12 above and below each account.
- Width 320. Height is the display's, between the menu bar and the Dock.

## Shape and light

- Square corners on the panel, bars and squares. 2px on the plan badge, 4px on tooltips. Dots are circles.
- Separation is hairlines, no shadows. Between the top row and the accounts, and between accounts, a 1px rule in `--accent` at 30%, edge to edge.
- The frame: a 1px border in the light, `--accent` at both sides and `--info` in the middle.
- The light is used for: the frame, the top row's gauge and status, limit bars under 70%, chart bars and the menu bar gauge. Nothing else.
- A limit bar's light spans the whole track, so a shorter fill shows only its near end. From 70% the fill is flat `--warning`, from 90% flat `--danger`.
- Chart bars are lit bottom to top. Past hours sit at 55%; the hour in progress is at full.

## Fit

No scrolling. The hourly charts stretch to fill spare height (24px minimum). If the accounts still overflow, the whole panel zooms out, down to 50%, until they fit. Dropped on another display, the panel takes that display's height and keeps the horizontal spot it was dropped at.

## Components

- Top row: 32px, also the drag strip. The gauge mark (three bars in the light), `usage` in `--text-dim`, then what the accounts are doing in the light: `5 active` or `idle`. At the right, refresh and hide, 14px, `--text-dim` at rest, `--text` on hover, `--accent` pressed.
- Account: a two-digit number in `--text-dim`, the account square, the name at 12px 600, truncated. The plan badge at the right, `--surface-raised`, 11px `--text-muted`, at most 96px wide.
- Activity: a 6px `--accent` dot and `3 active sessions`, or a `--text-dim` ring and `idle`. Then three figures in columns: `last hour`, `today`, `sessions`.
- Limit: name and percentage on one line, a 6px bar on a `--surface-raised` track with a 2px `--text` tick where the window's elapsed time is, then `resets in 2h 6m` and the pace.
- Hourly chart: `tokens per hour`, twelve bars on a `--border` baseline, then the first hour at left and `now · peak 61.3M/h` at right.
- Menu bar gauge: one bar per account (up to four), filled as far as its fullest limit, on a `--text-dim` track at 60%. Lit under 70%, then `--warning`, then `--danger`.
- App icon: the radio's tile, near-black with a cyan glow and a frame in the light, holding three gauge bars.

## Voice

Terse operator, lowercase. `resets in 2h 6m`, `pace: ~72% at reset`, `limit reached`. Errors say what failed and what to do: `token expired. start a claude session on this account, then refresh`. Menu items are the one place in title case, because the menu bar is macOS's, not ours.
