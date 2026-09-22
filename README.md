# WallLit

Animated wallpaper for Windows. Native, small, and quiet when there is nothing
to do.

The program does not ship anything Windows can already do by itself. There is no
bundled decoder, no web runtime, no service, no database. Windows already knows
how to decode H.264 on the GPU and how to compose the desktop — we use that.

## Principles

Recent decisions, limitations and planned work are in
[Architecture and evolution](docs/ARCHITECTURE.md).

1. **Native.** No web runtime.
2. **Light.** Do nothing when there is nothing to do.
3. **Hardware accelerated.** Video decoding belongs on the GPU.
4. **Offline.** No account, no cloud, no telemetry.
5. **Simple.** Choose. Position. Apply.

## Current state

Working:

- Surface attached to the Explorer wallpaper layer, behind the icons.
- Hardware decoding through Media Foundation, output straight into a D3D11
  texture.
- NV12 to RGB conversion and framing in the shader. No pixel goes through the
  CPU.
- Independent multi-monitor: video, framing and speed per monitor.
- Durable monitor identity, immune to Windows reordering.
- `config.json` with atomic writes.
- Settings window with a live preview, drag and zoom.
- Static frame applied as the Windows wallpaper: no flash at login.
- Import with probing, hardware conversion and caching.
- Per-monitor pause when a window covers the screen; sleep when the session is
  locked, the display is off, or the machine suspends.
- Tray icon, start with Windows, single instance, and settings reload without
  restarting the engine.
- Automatic reattach when Explorer recreates the wallpaper layer.
- Own library: imported media is copied inside the program.
- Picker grid with thumbnails, hover preview and multiple selection.
- Per-monitor playlist: several wallpapers in sequence, timed in video loops or
  in seconds.
- Options panel under the gear, in the top right corner.
- Eight transitions between wallpapers, drawn in a shader.
- Scheduling by time of day, using the system clock.
- Interface in Portuguese, English, Spanish and Russian, following the Windows
  language.
- Still images as wallpaper, alongside videos.
- Free resize with alignment guides, colour filters, and compositions made of
  several images and videos on the same screen.

Not there yet: recovery from a lost graphics device, and reacting to resolution
or monitor changes.

## Measurements

Reference machine: Windows 11 build 26200, primary monitor 1920×1080 @ 75 Hz.

| | 4K 60 fps (original) | 1080p 30 fps (imported) |
|---|---|---|
| Size on disk | 116.5 MB | 18.1 MB |
| CPU (whole system) | ~0.6 % | ~0.21 % |
| Working set | 93–250 MB | ~90 MB |
| Delivered frame rate | 60.00 fps | 30.00 fps |

Executables: **519 KB** for the engine, **633 KB** for the settings window. Time
to the first frame on screen, warm: **~264 ms**, with two monitors.

Neither opens a console. Nothing flashes on screen at login; called from a
terminal, they attach to it and diagnostic output keeps showing up.

Memory is dominated by the Windows decoder surface pool, not by our own
allocation: at 4K that is 10 NV12 surfaces of 12.4 MB each, ~124 MB right there.
At 1080p the same pool costs ~37 MB. It is the most concrete reason to normalise
media on import instead of decoding 4K forever on a 1080p monitor.

## The flash of the old wallpaper

The problem this project exists to solve is not solved by speed alone. The first
cold run cost 3.5 s against 278 ms warm — the difference is I/O loading the
Direct3D and Media Foundation DLLs. At login, when the disk is saturated, the
cold case is the normal case.

So we do not race the disk. When a wallpaper is applied, the first frame of the
video is rendered at the monitor resolution, with the same framing and the same
shader, and becomes the Windows static wallpaper. What shows up during the wait
is already the right frame, and the switch to the moving video is invisible.

The runtime cost is zero: the work happens once, on apply. Each monitor gets its
own frame, through `IDesktopWallpaper`.

## Import

The runtime needs to be excellent at one thing only: playing H.264 at the monitor
resolution. Instead of teaching the engine every format in existence, we
normalise on import.

```
import
   ↓
probe
   ↓
already in the library? ── yes → reuse
   │
   no
   ↓
already suitable?
   ├─ yes → copy as is, no re-encoding
   └─ no  → convert once
```

Suitable means H.264, no larger than the monitor, and up to 30 fps. An audio
track present does not disqualify it: the engine never selects the audio stream,
so it costs neither CPU nor memory — and re-encoding a good file would only
remove quality.

**An imported file always becomes a copy inside the program**, under
`%APPDATA%\WallLit\wallpapers`. This is not a cache: it is the copy the wallpaper
depends on. Deleting the original, renaming it, or unplugging the drive it came
from has no effect once the import has finished.

Names join a readable part to a fingerprint of the contents:

```
hollow-knight-1-moewalls-com-289f114d.mp4
```

The readable part is there so that whoever opens the folder recognises what they
are looking at; the fingerprint guarantees that two different files never fight
over the same slot, and that the same file imported again lands exactly on what
is already there.

Conversion uses the Windows H.264 encoder, hardware accelerated. No FFmpeg.
Measured on this machine:

```
hollow-knight.mp4   3840×2160  60 fps  116.5 MB
                          ↓  8.4 s
cache/5f50c5e0.mp4  1920×1080  30 fps   18.1 MB
```

A file that already fits is stored once and serves any monitor; target parameters
only enter the equation when there is a conversion.

The settings keep both paths: the library one, which the engine opens, and the
original file, which the interface shows as the name. The engine never converts
anything at login — preparing media is the interface's job.

## Images

An image goes into video memory once and stays there. There is no live decoder,
no frame clock, no next frame to schedule — and when there is no next event, the
engine schedules no wake-up at all:

```
two monitors with a still image    0.0 % of one core   48 MB
two monitors with 1080p30 video    5.1 %               131 MB
```

The zero was measured over three eight-second samples, not rounded.

JPEG, PNG, BMP, TIFF, WebP, HEIC and AVIF all go through the Windows image
decoder. An image larger than the monitor is scaled down on import and stored as
PNG: a camera photo at full size would occupy tens of megabytes of video memory
to show up on a two-megapixel screen. An image that already fits is copied byte
for byte, without re-encoding.

Framing goes through the same shader as video, so dragging and zooming work the
same way. Verified: colours reach the screen with zero difference from the file.

GIF is deliberately left out of that list — a GIF is usually animated, and the
import converts it to video so it plays like any other wallpaper.

## Doing nothing when there is nothing to do

Each monitor has its own state, and they exist for a single purpose: not spending
energy on what nobody is looking at.

| | when | what happens |
|---|---|---|
| Playing | desktop in view | decodes and draws |
| Paused | a window covers **that** monitor | stops everything; the last frame stays on screen |
| Sleeping | session locked, display off, machine suspended, or pause requested from the tray | stops everything |

The granularity is per monitor: a fullscreen game on one screen does not freeze
the other.

Stopped does not mean hidden. When no monitor is playing there is no next frame
to schedule, the loop waits with no timer at all, and the thread only wakes if
the system sends a message. Measured with both monitors covered:

```
two monitors playing    5.1 % of one core   (0.42 % of the system)
two monitors covered    0.0 %
```

None of this is polled. Windows tells us when the session is locked, when the
display turns off and when the machine is going to sleep; and it tells us,
through an event hook, when a window appears, disappears, changes size or comes
to the front. The coverage check runs at those moments and at no other.

Two traps that were worth the time to find, and that `--probe` now diagnoses on
its own:

- Overlays from graphics drivers, game platforms and capture tools create windows
  the size of the whole screen that are almost entirely transparent.
  Geometrically they cover everything; visually, nothing. Without handling this,
  the wallpaper would stay paused forever on any machine with one of them
  installed.
- A game that is closed disappears without ever emitting the window-hidden event.
  Without also listening for the destroy event, the wallpaper would stay frozen
  until something else happened on screen.

## Tray and startup

The tray icon belongs to the **engine**, not to the interface. Closing the
settings window still ends that process entirely, and what stays in the tray is
the engine, which was already running. From the user's point of view the program
was minimised; from memory's point of view, nothing was kept alive that was not
already.

The icon menu opens the settings, pauses, and quits. If Explorer restarts and
takes the tray with it, the icon puts itself back.

Start with Windows uses the current user's `Run` key: no administrator
privileges, no service, no scheduled task, and visible in Task Manager > Startup,
where it can be turned off without opening WallLit. If the program folder is
moved, the stored path is corrected the next time the settings open.

There is only one engine per session, guaranteed by a named object. When the
settings are saved, a running engine is notified and rereads everything without
restarting; if there is none, it starts right away.

## Architecture

Two executables. The engine has no interface; the interface is not resident.

```
walllit.exe           engine: settings, monitors, decoding, drawing
walllit-settings.exe  interface: only exists while the window is open
```

```
src/
├── lib.rs       core shared by both executables
├── config.rs    config.json, atomic writes, application folders
├── desktop.rs   integration with the Explorer wallpaper layer
├── autostart.rs start with Windows
├── display.rs   monitors and durable identity
├── icon.rs      SVG paths turned into geometry
├── import.rs    probing, hardware conversion and library
├── language.rs  language detected from the system and text table
├── instance.rs  single instance and notifications between processes
├── library.rs   index of imported wallpapers and thumbnails
├── occlusion.rs which monitors are covered
├── poster.rs    static frame applied as the Windows wallpaper
├── renderer.rs  Direct3D 11, colour conversion, framing
├── session.rs   lock, power and display state
├── stats.rs     high resolution clock and instrumentation
├── tray.rs      tray icon and menu
├── ui.rs        immediate mode widgets over Direct2D and DirectWrite
├── video.rs     Media Foundation and hardware decoding
├── shaders/     HLSL compiled at build time
└── bin/
    ├── walllit.rs           engine
    └── walllit-settings.rs  interface
```

The interface uses the same renderer and the same decoder as the engine. The
preview is not an approximation of the result: it is the result, drawn in a
window instead of on the desktop.

Icons are SVG paths converted into Direct2D geometry. An icon font would mean
choosing between two different fonts depending on the Windows version, and a
bitmap would look jagged on scaled displays.

### The wallpaper layer

Explorer has used three different arrangements for the window where the wallpaper
is drawn, and versions in the field still use all of them. On this Windows 11 the
`WorkerW` is a **child** of `Progman`, below `SHELLDLL_DefView`:

```
Progman
 ├ SHELLDLL_DefView   icons
 └ WorkerW            wallpaper layer  <- we attach here
```

On Windows 10 the `WorkerW` is usually a top-level sibling of the icon window.
`desktop::find_anchor` tries the arrangements from the most specific to the most
generic and falls back to `Progman` as a last resort. None of this changes
Explorer permanently: if the process dies, the window dies with it and the
desktop returns to normal on its own.

### The layer that vanishes underfoot

Explorer destroys and recreates the wallpaper layer in more situations than it
seems: on restart, on a theme change, and — the case that happens every day —
when a static wallpaper is set, which is exactly what WallLit does on apply.

Since our surfaces are child windows of it, they die with it. The symptom is
cruel to diagnose: the preview stays perfect, Apply says it saved, and the
desktop shows a still image — which is the static frame, identical to the first
frame of the video.

That is why the engine checks, on every window change and on every reload,
whether it is still hanging from the right tree, and reattaches itself when it is
not. The same check covers an Explorer restart.

### Monitor identity

Settings are indexed by hardware, not by the number Windows assigned:

```
DISPLAY-AOC2402-5&1a2b3c&0&UID4352
```

Monitor model and the output on the card it is plugged into. Swapping two cables
does not swap the wallpapers.

## Usage

Interface:

```
walllit-settings [video.mp4]
```

A video passed as an argument comes in already chosen for the current monitor.

Engine:

```
walllit                                     uses the saved settings
walllit <video.mp4> [--monitor N] [--mode fill|fit|stretch|center|custom]
walllit --apply <video.mp4> [--all] [--speed 0.5]   prepares, saves and applies
walllit --import <video.mp4> [--monitor N]  only prepares, and shows the result
walllit --autostart on|off|status           start together with Windows
walllit --probe                             inspects desktop and monitors
```

`--apply` does the whole cycle: imports the media, writes the settings and sets
the static frame. It is the same thing the interface does when you click Apply.

`--probe` dumps the desktop window tree, the monitor list, and what is covering
each one at this instant. It is the first thing to ask for in a bug report: it
shows which Explorer arrangement the machine is using and, when a wallpaper
appears paused for no reason, which window the system believes is in front.

`--stats` prints the time of each startup stage, the real frame rate, CPU and
memory. Without the flag, no measurement is taken.

## Building

Requires Rust (target `x86_64-pc-windows-msvc`) and the Windows SDK, which
provides `fxc.exe`, which compiles the shaders, and `rc.exe`, which compiles the
icon and the version information. Everything is embedded in the executables, so
there is no dependency on `d3dcompiler` or on external files at runtime.

```
cargo build --release
```

If the SDK is in a non-standard location, point the `FXC` and `RC` variables at
the corresponding executables.

## License

MIT. See [LICENSE](LICENSE).
