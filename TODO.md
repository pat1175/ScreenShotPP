# TODO — ScreenShotPP improvements

Sorted by priority / effort. Each item keeps its open technical question where one exists.

## High priority (stability / blockers)

- [x] **Fix the Windows capture crash (0xc0000409, WGC FFI)** — shipped in v0.2.2
      Root cause: `windows-capture` panicked when driven from the main GUI thread
      (it already owns a DispatcherQueue / COM apartment), and the panic crossed the
      FFI boundary, aborting the process. Fixed by running WGC capture on a dedicated
      thread, with `catch_unwind` turning any residual panic into an `Err`
      (`capture_win.rs`).
  - [x] Regression test for the dedicated-thread / panic-isolation path — the
        duplicated thread-spawn + `catch_unwind` logic in `capture_at_point` and
        `capture_foreground_window` was extracted into one helper, `run_isolated`,
        directly unit-tested with panicking/Ok/Err stub closures (no real WGC
        session needed). CI-verified on `windows-latest`.
- [ ] **Sign the Windows binary** (SignPath Foundation application)
      Removes the SmartScreen warning and unblocks real-world adoption.
      Note: the existing `windows-x86_64` signature in the release workflow is the
      Tauri updater signature (update integrity), not Authenticode code signing.
      Blocked on an external dependency: the SignPath application can only be filed
      after the first public release.

## Planned features

- [x] **Delayed capture / timer** — shipped in v0.3.0, Windows fixes in v0.3.1
      Dedicated configurable shortcut (default `⌘⇧3` / `Ctrl⇧3`) starts a
      cursor-following, click-through countdown (configurable delay, configurable
      cancel key), then captures and opens the editing overlay. Windows follow-ups
      in v0.3.1: fixed a global-shortcut re-entrancy freeze, a missing countdown
      window capability (number stuck at 3), and a stray window shadow.

- [x] **Capture a window that extends beyond the monitor bounds** — implemented on
      `feature/window-overflow-capture`, tagged `v0.4.0-rc.2`; not yet merged to
      `master`.
      Impl ended up native-API-based rather than `xcap::Window`: WGC `OneShot` on
      Windows (`capture_win.rs`), `CGWindowList` on macOS, overflow detected via the
      unclipped global window rect, editor gets a letterboxed background + re-init
      on overflow-window select.
      Remaining before merge: real-device Windows/macOS QA (multi-monitor,
      mixed-DPI) — flagged as deferred in code review, not yet exercised outside CI.

- [ ] **Video / short GIF recording**
      Large feature (competes with ShareX / CleanShot).
      Open questions: encoder (bundled ffmpeg? OS API?), binary size, continuous
      capture performance, output format (mp4 / gif / animated webp).

- [x] **Adjust the capture area after selection** — shipped in v0.5.1.
      Each of the four sides can be dragged independently through a small white
      circular handle centered on that side.

- [ ] **Capture history**
      Persist each copied/saved capture to a dedicated folder plus an index, with a
      small gallery window to re-copy or re-open.
      Open questions: storage location, size limit, automatic pruning.

- [ ] **Highlighter** — small
      Semi-transparent yellow rectangle using `globalCompositeOperation = 'multiply'`
      in the Konva editor. Content under the stroke stays readable.

## Large / long-term

- [ ] **Native Rust UI (drop the webview)** — in test on `feat/egui-native`
      Full egui port: one winit loop, one glutin/egui window per app window
      (settings, overlay, countdown, recording HUD, recorder), `tray-icon` +
      `global-hotkey` directly, CPU export (tiny-skia), ffmpeg-decoded video preview
      in the recorder. No webview, no Node toolchain; `src-tauri/` became `app/`.
      Remaining before merge: real-device Windows/macOS QA (multi-monitor,
      mixed-DPI, IME), first cargo-packager release run (DMG/NSIS + updater
      artifacts), RAM comparison against v0.5.1.

## Out of scope (decided 2026-06-30)

- Arbitrary / background window picker — impossible on a frozen image, and
  per-window capture is too heavy for the benefit.
- Gaussian blur — adds almost nothing over the existing mosaic redaction.
- "Auto" step counter — already provided by the existing numbered bubbles.
- Grid snapping for shapes — low value for a fast capture workflow.
- SVG / vector export — not needed.

## Quality (non-feature)

- [x] Extract pure logic into a GUI-free `screenshotpp-core` crate — geometry,
      countdown, and settings now unit-test locally without the Tauri toolchain
      (`cargo test -p screenshotpp-core`).
- [x] Split CI into a fast pure-logic job (~45 s) and the full platform builds;
      Windows uses the D: drive for cargo.
- [ ] Integration tests for the overlay (`app/src/ui/overlay.rs` interaction flow
      is uncovered; only its pure logic and export are unit-tested).
- [ ] Improve permission / OCR error messages.
