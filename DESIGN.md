# Claude Usage styling

Compact, dark desktop usage monitor with a translucent navy background.

- Background: navy `#040C1C` at 90% opacity (230/255), letting the desktop show through while keeping the panel readable. Applies to the main window, including its drag strip.
- Compositing: transparent native window and transparent renderer clear color; the central panel supplies the navy tint.
- Cards: existing opaque gray `#1E1E1E`, preserving legibility of usage details.
- Type: existing egui default font with a compact 9.5–18px scale; larger bold percentages emphasize usage.
- Color: existing per-account blue, purple, teal, and orange accents distinguish accounts; muted labels use gray `#8C8C8C`.
- Spacing: existing dense layout, with 4–6px within groups and 16px between account cards.
- Finish: existing thin borders separate the window and cards.

- Window priority: overlapping-window icon next to the desktop pin; green and filled when enabled, gray and outlined when disabled. Remembers the setting; defaults to enabled.
- Icon feedback: pointer cursor, light hover, white pressed state, and a 2px keyboard focus outline make controls identifiable.
