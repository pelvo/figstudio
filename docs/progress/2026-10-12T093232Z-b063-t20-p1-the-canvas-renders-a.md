- 2026-10-12 — T20 P1: the canvas renders a margin (half the canvas per side, capped by texture
  limit, u16 and 32 Mpx) and requests a new frame only near the margin's edge or on zoom/size/page
  change; 6 unit tests seen failing under 5 mutations; scripted pan showed no empty edge and one
  request per ~320 pt of pan. Not verified: real hand-tool drag (no input tool), web by eye.
