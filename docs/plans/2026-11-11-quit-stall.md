---
branch: fix/quit-stall
pr: 87
title: fix(app): quit without freezing the window on big documents
---

## P3 — Desktop app (`apps/figstudio`)
- [x] Quitting leaves the open documents to the OS instead of freeing them node by node on
      the UI thread while the window is still up (asserted in
      `a_space_change_is_saved_when_its_popover_is_replaced_its_file_closes_or_the_app_quits`)
- [x] Timed Cmd-Q on two `synth` stress files (0.69M and 1.9M nodes, 4–5 GB resident)

## Progress

### 2026-11-11T092246Z quit-stall
- What changed and how it was checked: quitting with big files open showed the beach ball
  for seconds. eframe drops the app before it closes the window, and a sample of the main
  thread during quit spent 64 of 73 samples in `Tab` → `Arc<Document>` → `Val` drops (`free`).
  `on_exit` now forgets the tabs; the workspace is already saved by `save`, which eframe calls
  first, and no `Drop` in a tab has an effect outside the process. Cmd-Q to process exit on
  two `synth` stress files (4–5 GB resident): about 2 s before (0.95–2.4 s), about 0.4 s after
  (5 of 6 runs; one cold run 1.16 s). The new assert failed without the change and passes with
  it.
- Not verified: real-world private files; closing one big tab (⌘W) still frees its document on
  the UI thread; Windows and Linux quit timing; a load that finished but was not yet taken
  from the worker channel at Cmd-Q still frees its document on the UI thread (only `tabs` is
  forgotten).
