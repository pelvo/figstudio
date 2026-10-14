# Native menu bar plan (muda)

Goal: the desktop app uses the OS menu bar on macOS and Windows, built from one menu
definition shared with the drawn egui menu, so the two never drift.

## Decisions

1. **Linux keeps the drawn egui menu.** muda needs a GTK window on Linux; eframe's winit
   window is not one. The browser build keeps it too.
2. **One `unsafe` exception on Windows** for `Menu::init_for_hwnd`, allowed locally with
   `#[expect(unsafe_code, reason = ...)]` and a `SAFETY` comment, as in `doctor.rs`.
3. **PR #17** (View menu stays open on inside clicks) is reused for this work; its title and
   description change to match.

## Design

- **`menu.rs`: one definition.** An `Action` enum (`Open`, `ExportPage(scale)`,
  `ExportSelection(scale)`, `CloseTab`, `Quit`, `ZoomFit`, `ActualSize`, `ToggleLayers`,
  `ToggleProperties`, `AgentAccess`, `About`, web-only `CjkFont(v)`) and a function that builds
  the menu tree (label, shortcut, enabled, checked) from app state. `App::apply(Action)` is
  the only place menu clicks and keyboard shortcuts are handled.
- **Two renderers.** `menu_egui.rs` (Linux, web, headless tests, fallback when native attach
  fails) and `menu_native.rs` (`cfg(any(target_os = "macos", windows))`): builds the muda
  `Menu`, keeps it in `App`, updates enabled/checked only when they change.
- **Events.** `MenuEvent::set_event_handler` sends the item id over a channel and calls
  `ctx.request_repaint()`; each frame drains the channel into `apply`.
- **No widgets in menus.** Agent access (toggle, remote, address, Copy token) moves to an
  **Agent access…** window; the Help text moves to an **About figstudio** window (macOS: in
  the app menu).
- **macOS.** `init_for_nsapp()` in the `run_native` creation closure. App menu (About, Hide,
  Hide Others, Quit), File, View, Window, Help. Quit is a custom item that closes through
  eframe, not muda's predefined Quit, so the app shuts down cleanly.
- **Windows.** `init_for_hwnd` with the HWND from the creation context. winit's message loop
  does not call `TranslateAcceleratorW`, so menu accelerators are display-only and the app's
  own `shortcuts()` keeps doing the work.
- **Dependency.** `muda` (tauri-apps, maintained, widely used) for macOS and Windows targets
  only, `default-features = false` so no GTK.

## Lanes

Each lane is one commit and passes `scripts/check.sh`.

| # | Lane | Files | Done check |
|---|---|---|---|
| 0 | Spike (throwaway branch): attach muda in eframe on macOS; event delivery; whether a menu accelerator (⌘O) also reaches egui; Quit | scratch | Notes on each point, no merge |
| 1 | Menu definition and `Action` refactor, no visible change | `menu.rs`, `menu_egui.rs`, `main.rs` | Existing UI tests unchanged and green; new tests for enabled/checked rules |
| 2 | Agent access… and About windows; no widgets left in menus | `mcp_host.rs`, `mcp_web.rs`, `mcp_off.rs`, new window file | UI test types an address in the window and starts the remote server |
| 3 | macOS native menu; egui bar hidden once attached | `menu_native.rs`, `main.rs`, `Cargo.toml` | Manual checklist on macOS; `check.sh --cross` |
| 4 | Windows native menu | `menu_native.rs` | `cargo check --target x86_64-pc-windows-msvc` in `check.sh --cross`; manual run on Windows |
| 5 | Docs: PLAN, IMPLEMENTATION (menu architecture), TESTING (manual menu checklist) | docs | All checks pass |

## Spike findings

- muda 0.21.2 is the current release (Rust 1.90 minimum); it works with eframe
  0.36 / winit 0.30 without GTK when default features are disabled.
- A scratch app attached `init_for_nsapp()` in the creation closure. Cmd+O delivered
  one menu event to the egui frame via a channel plus `request_repaint()`.
- egui observed only the O key **release**, not a press. Still skip egui handling of
  menu-owned accelerators when attached on macOS to prevent duplicate actions.
- Custom Cmd+Q delivered a menu event; `ViewportCommand::Close` shut down the scratch
  app cleanly. No scratch code is included in the repository.
- Existing bare-HTTP tests attempt to bind the live agent port. During these lanes
  the gate excludes those two tests and (lanes 0–1 only) the pre-window MCP UI test;
  the latter is replaced by ephemeral-port window coverage in lane 2. Exclusions are
  local-only and are not part of `scripts/check.sh`.

## Implementation checks

- Lanes 0–4 passed the full local gate, with the exclusions above. macOS native
  attachment, View→Agent access, single Cmd+O picker, Cmd+W disabling exports and
  clean Cmd+Q were observed in an extra public-fixture instance.
- Windows target installed and `cargo check -p figstudio --target
  x86_64-pc-windows-msvc` passed (host CPU flag warnings only). Windows runtime,
  complete macOS checklist and browser GUI remain unverified. The browser release
  build passed; its pre-existing unused bridge import warning remains.
- Lane 1 preserves existing agent/help widgets; lane 2 moves them into windows,
  including the browser bridge controls. No widget menus remain on desktop.

## Out of scope

Context menus (`copy_menu.rs`, `region_copy.rs`) stay egui. The web build keeps the egui menu.

## Risks

- **Double shortcuts on macOS**: the native accelerator and egui may both see ⌘O. Lane 0
  answers this; the likely fix is to skip egui handling for shortcuts the menu owns.
- **Native menus are not testable headless**: lanes 3 and 4 rely on manual checklists.
- **No Windows machine here**: lane 4 is cross-checked only until someone runs it.
