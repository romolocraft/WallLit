# WallLit architecture and evolution

Goal: choose a wallpaper, apply it, and leave the program alone. Being light
means little CPU/GPU work, memory proportional to the media, and predictable
behaviour. Executable size on its own does not measure that.

## First stage — implemented on 2026-09-21

- **Pause policy isolated in `src/playback.rs`.** An inactive session or a manual
  pause takes precedence. Coverage only pauses when `pause_when_fullscreen` is
  enabled. Before, the preference was saved but ignored by the engine.
- **Coverage tied to monitor identity.** Failing to create the surface for one
  screen no longer shifts the coverage state of the others.
- **Reload distinguishes an error from an empty list.** Invalid JSON, an unknown
  version or invalid parameters preserve the engine's current state. Valid
  settings with no wallpapers remove the surfaces and leave the engine available
  in the tray for the next apply. If the desktop anchor is unavailable during the
  reload, the request is rejected before the previous state is discarded; a new
  apply is required.
- **Validation on read and on write.** Positive and finite speed and scale;
  finite offsets; non-empty path; positive loop counts; a duration of at least
  one second; times within the day. UTF-8 with BOM and migration from the old
  format are still accepted. Schedule references beyond the list keep the
  previous behaviour: the engine clamps to the last available item.
- **Shared persistence in `src/storage.rs`.** Settings and library use a
  temporary file exclusive to each writer, synchronisation, and replacement by
  Windows. The library is no longer truncated directly on write. This does not
  solve logical conflicts: two interfaces can still save different versions, and
  the last write wins.
- **A minimised interface suspends previews and presentation.** With no import
  running, it waits for messages with no timer. With an import running, it checks
  for completion every 100 ms without drawing the window. On restore, it
  re-anchors the preview clocks. The final processing of an import, including the
  thumbnail, can still use the GPU. The progress bar no longer requires 60
  redraws per second; videos and interactions keep their own cadence.

No production dependency was added. The JSON format stays at version 1. The
organisation of the executables has not been fully refactored yet.

## Limits that remain

- Recovery from a lost graphics device, and monitor/resolution changes.
- Apply still recreates every surface. Generating the static wallpaper can
  recreate the Explorer layer: incremental updates must take that into account.
- A visible, idle interface still has a timer; the next stage should use
  event-driven invalidation and deadlines only for animation and hover.
- `Config::load` still falls back to defaults at startup when there is an error.
  The protection from this stage covers the engine reload; the interface still
  needs to offer an explicit recovery before allowing damaged settings to be
  overwritten.
- The library still falls back to defaults silently if its index cannot be read.
  Atomic writes protect new writes, but do not rebuild data already lost.
- Synchronous decoding can block the shared loop. Measure before introducing
  callbacks or threads; if needed, use a queue bounded to a few frames.
- A media failure in a playlist can make the rotation insist on the same item.
  Persistent drawing failures also need progressive backoff and bounded
  diagnostics, so they do not produce continuous retries.

## Intended structure, in stages

Keep a single Rust project and the two executables. Extract responsibilities as
they are changed, avoiding moving files purely for organisation.

| Part | Responsibility | Must not do |
|---|---|---|
| Model and policy | Pause, playlist, schedules, validation | Call graphics APIs |
| Engine | Per-monitor state, deadlines, recovery | Import or convert media |
| Media | Library, import, decoding and thumbnails | Decide user preferences |
| Interface | Edit settings and show the preview | Duplicate the CLI apply flow |
| Windows integration | Desktop, displays, session, tray, GPU | Concentrate playlist rules |
| Persistence | Read, validate and write files | Turn a read failure into a successful reload |

Next extraction: a shared apply flow for the interface and the CLI. Separate
preparing media, generating posters, persisting and notifying the engine,
reporting which stage failed. Avoid declaring full success just because the JSON
was saved.

## Prioritised roadmap

### 1. Reliability before new features

- Handle display changes by event, re-enumerate identities and preserve the
  settings of disconnected monitors. Do not poll the topology.
- Recover the GPU and Explorer with spaced retries and a message rate limit. Keep
  a static image as a fallback while unavailable.
- Skip invalid media in a rotation and stop fast retries when every item fails.
  Isolate the failure per monitor.
- Distinguish civil time (scheduling) from playback time (playlist). Re-evaluate
  schedules when the session resumes and when the system clock changes.
- Offer recovery for damaged settings/index without silent overwriting.

### 2. Less work and less memory

- Update only the monitors that changed. Language or pause-policy changes must
  not reopen decoders; framing should update graphics parameters.
- An event-driven visible interface, including a single deadline to start the
  hover preview and failure handling without continuous retries.
- A thumbnail cache with a memory limit, discarding the least used.
- Consider releasing decoders after a long pause. Compare the memory saved
  against the resume time before choosing the delay.
- Consider the native Windows wallpaper for a single image with no transition or
  schedule. Only adopt it if it keeps the framing and the experience; avoid two
  divergent visual implementations to save a few megabytes.

### 3. Proposed additions — not implemented yet

| Addition | Benefit | How to keep it simple |
|---|---|---|
| Pause on battery / energy saver | Useful on laptops | One preference, driven by Windows notifications |
| Next wallpaper from the tray | Switch without opening the window | Reuse the same playlist advance rule |
| Search and favourites | Find items in large libraries | Metadata in the JSON; no database or resident indexer |
| Space used and orphan cleanup | Avoid unbounded growth | On demand; show what will be deleted and check references from every monitor, including disconnected ones |
| Import keeping the original quality | Portability without mandatory conversion | Separate copying to the library from optimising resolution/codec |
| Diagnostics reachable from the interface | Explain why a screen is paused or failed | Per-monitor state and a bounded local log; no telemetry |

Before identifying media by path in more places, consider a stable `MediaId` in
the index. It makes favourites, per-resolution variants and safe cleanup easier.
That migration must keep opening existing settings.

Not a priority in this phase: accounts, cloud, a store, a plugin system, web
wallpapers or an extra resident service. Every new feature must justify its cost
and work without continuous effort when it is not being used.

## Verification

Automation: `cargo test --all-targets` covers the pause preference, session
precedence, per-monitor association after a partial failure, migration, invalid
JSON, invalid parameters, schedules across midnight, and replacement of the JSON
file. The persistence tests use a temporary directory, not the real settings.

Result of this stage: 8 tests passed; the production build completed with
`cargo build --release --target-dir target/validated`. The alternative folder
avoids replacing `target/release/walllit.exe`, which was locked. The new
executables are in `target/validated/release`. The visual script below has not
been run yet.

Manual script for Windows:

1. With a video and a window covering the monitor, toggle the pause preference
   and apply.
2. Use two monitors, with invalid media on the first; check that the second
   screen gets its own coverage state.
3. With the engine running, try to reload invalid JSON; confirm playback is
   preserved. Then apply an empty list and apply valid media again.
4. Minimise/restore the interface with a video, an image, a grid preview and an
   import. Confirm it resumes without speeding up to make up for the minimised
   period.
5. Measure process CPU, private memory, working set, GPU usage and time to first
   frame, with the same media, resolution and number of screens.

Do not attribute a percentage gain to the changes without running those
measurements. The historical numbers in the README are not benchmarks of this
stage.
