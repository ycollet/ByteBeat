# Bytebeat Composer (native Rust)

A standalone desktop bytebeat studio inspired by the layout and workflow of
[Dollchan Bytebeat Composer](https://github.com/SthephanShinkufag/bytebeat-composer).
This is an independent implementation, not a web wrapper: one executable, no
browser, server, network connection, or external song files required.

For the language reference, playback semantics, and complete example programs,
see [DOCUMENTATION.md](DOCUMENTATION.md).

## Run

Install a current stable Rust toolchain, then:

```sh
cargo run --release
```

To distribute it, run `cargo build --release` and copy `target/release/bytebeat`
(`bytebeat.exe` on Windows). The executable embeds the GUI, JavaScript engine,
fonts, and eight original demo songs. It still uses the operating system's
windowing and audio libraries.

On Debian/Ubuntu, install the native build dependencies first:

```sh
sudo apt install build-essential pkg-config libasound2-dev libudev-dev \
  libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxkbcommon-dev \
  libwayland-dev libgl1-mesa-dev
```

macOS requires the Xcode command-line tools. Windows requires the MSVC Rust
toolchain and Visual Studio C++ build tools.

## Compose

- **Editor:** enter a JavaScript expression, then click **Apply & play** or
  press **Ctrl+Enter** / **Cmd+Enter**. Edits are applied explicitly, so incomplete
  typing does not interrupt playback.
- **Playback:** Play/Pause, Stop (reset to zero), Restart, volume, sample-rate
  presets, and custom rates from 1,000 to 192,000 Hz. Playback starts paused at
  25% volume; use a comfortable listening level.
- **Visualization:** stereo waveform, logarithmic FFT spectrum, or both.
  Click the scope to toggle playback. Zoom adjusts the waveform's visible window.
  The spectrum analyzes the latest 2,048 output frames using a Hann window.
- **Library:** search the included demos, load a song, or save your expression to
  **My songs**. Saving the same title replaces that local song; **x** deletes it.
  The editor, settings, and local songs persist through egui's native application
  storage when the application saves/exits. They are not written beside the binary.

## Upstream catalog and file loading

The downloaded catalog lives in `data/`. Download or resume it with Python 3
(standard library only):

```sh
python3 scripts/download_catalog.py
```

The downloader retrieves the live catalog from Dollchan, not the GitHub
repository's empty placeholder. It writes one complete `.js` source and a `.json`
metadata sidecar per song under `data/songs/`, plus `data/catalog.json`, the raw
`data/all.gz` index, and `data/UPSTREAM-LICENSE`. Original code is preferred;
minified/formatted sources are fallbacks. Truncated previews are never treated as
complete programs. Metadata retains the upstream author, source URLs, tags, and
original catalog entry, with a SHA-256 digest of each downloaded source.
Downloads use four concurrent workers, retry failed requests, reuse verified
local sources when their metadata is unchanged, and exit nonzero if any song
could not be downloaded. `--refresh` refetches all external sources. Use
`--limit 20` for a small initial collection or `--output PATH` for another location.

The GUI automatically adds `data/catalog.json` to the library when it exists in
the current working directory. Alternatively, specify a catalog or a song:

```sh
cargo run --release -- --catalog data/catalog.json
cargo run --release -- --file data/songs/e295af172c127b1f527c823ad0aeaeda.js
cargo run --release -- --file my-song.js --mode floatbeat --sample-rate 44100
```

A matching `.json` sidecar supplies a file's title, author, mode, and sample rate.
Without one, the defaults are Bytebeat at 8,000 Hz. Explicit CLI options override
metadata. Fractional sample rates are retained. File loading opens the editor
paused; select Play when ready. The optional catalog is **not embedded** in the
executable: distribute `data/` alongside it if you want these additional songs.
Without it, the built-in library still works offline.

### Headless compatibility checks and console logs

`--check` opens neither a window nor an audio device. It compiles each selected
program in a fresh runtime and renders from `t = 0`, using the same engine and
128-frame render blocks as live playback:

```sh
# Check one file with its metadata.
./target/release/bytebeat --file data/songs/e295af172c127b1f527c823ad0aeaeda.js --check

# Check every catalog entry for one second and retain a JSON report and console log.
./target/release/bytebeat --catalog data/catalog.json --check --seconds 1 \
  --report data/check-report.json > data/check.log 2>&1

# Check an expression that has no sidecar.
./target/release/bytebeat --file my-song.js --mode funcbeat --sample-rate 48000 \
  --check --seconds 5
```

The checker exits **0** when every song passes and **1** for loading,
compilation, or rendering failures. It continues to later songs after a failure.
With both `--file` and `--catalog`, check mode checks just the file; GUI mode
loads the file and the library. `--seconds` accepts 0.01–60 seconds per song.
The report records each song's mode/rate, failing stage, JavaScript diagnostic,
sample position in the error text, source samples checked in completed blocks,
peak normalized amplitude, and elapsed time. GUI audio errors also appear in
console logs. Set `RUST_LOG=bytebeat=debug` to control logging.

A passing check means the requested **initial interval rendered without errors**,
not that the entire song was verified, that its audio matches a browser
bit-for-bit, or that it can meet real-time deadlines. NaN/infinity, deliberate
`throw`-based console songs, missing browser APIs, resource limits, and rates
outside 1,000–192,000 Hz are reported as failures rather than silently ignored.
Downloaded code is third-party JavaScript; the runtime has no filesystem/network
bindings, but its memory/time limits are not an operating-system security sandbox.
Keep upstream attribution and notices when sharing downloaded material.

### Expression modes

| Mode | Time variable | Output |
| --- | --- | --- |
| Bytebeat | `t`: sample index | Truncated, wrapped unsigned 8-bit values (0–255) |
| Signed Bytebeat | `t`: sample index | Truncated, wrapped signed 8-bit values (-128–127) |
| Floatbeat | `t`: sample index | Floating point, clipped to -1…1 |
| Funcbeat | `t`: seconds | Function returning floating point, clipped to -1…1 |

For example:

```js
((t * ((t >> 12 & 3) + 2)) & 127) + ((t >> 4) & 63)
```

Return `[left, right]` for stereo. `sampleRate` is the formula's selected sample
rate. `Math` members are available directly (`sin`, `PI`, `random`, etc.); `int`
is an alias for `Math.floor`. Funcbeat source is a function body that runs once
without arguments and must return a function. That returned function receives
time in seconds and the source sample rate:

```js
let phase = 0;
return (t, sampleRate) => {
  phase += 2 * PI * 220 / sampleRate;
  return 0.3 * sin(phase);
};
```

The embedded QuickJS runtime supports JavaScript, including persistent globals,
closures and arrays. `window` and `self` are aliases of `globalThis` for songs
that use them to store state; they **do not provide browser APIs**, DOM, timers, filesystem
access, or network access. Expressions are evaluated sequentially on an audio
producer thread. Each program has a 32 MiB JS memory limit and a 100 ms execution
budget per render block (and during initialization). Syntax/runtime errors,
non-numeric results, NaN, infinity, and timeouts are displayed in the status bar
and stop playback instead of being silently converted to audio.

## Architecture and current scope

`eframe`/egui draws the native GUI; `rquickjs` evaluates formulas; CPAL drives
the default output device; a bounded lock-free ring buffer separates JavaScript
from the real-time audio callback. `rustfft` supplies the spectrum. Formula
samples are converted to the device rate with zero-order hold (intentionally
preserving the classic stepped sound); rates above the device rate skip output
samples but still evaluate each source sample, preserving stateful formulas.
The scope and counter show generated audio, ahead of audible output by up to
the 2,048-frame queue plus the device buffer.

This initial version has a plain monospaced editor and the core
compose/play/visualize workflow, with an optional downloaded upstream library.
It does not import share URLs, export recordings, or include all of the web app's
editor themes and visualization modes. No upstream application code is bundled.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
python3 -m unittest discover -s scripts -p 'test_*.py'
```
