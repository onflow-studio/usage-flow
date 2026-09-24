# Claude Usage styling

Compact, dark desktop usage monitor with a translucent navy background.

- Background: navy `#0A192F` at 85% opacity (217/255), letting the desktop show through while keeping the panel readable. Applies to the main window, including its drag strip.
- Compositing: transparent native window and transparent renderer clear color; the central panel supplies the navy tint.
- Cards: existing opaque gray `#1E1E1E`, preserving legibility of usage details.
- Type: existing egui default font with a compact 9.5–18px scale; larger bold percentages emphasize usage.
- Color: existing per-account blue, purple, teal, and orange accents distinguish accounts; muted labels use gray `#8C8C8C`.
- Spacing: existing dense layout, with 4–6px within groups and 16px between account cards.
- Finish: existing thin borders separate the window and cards.
