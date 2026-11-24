# Bytebeat Composer documentation

This guide covers installation, application controls, the bytebeat language,
catalogs, terminal tools, and development. See the [README](README.md) for a
quick introduction.

Bytebeat Composer generates sound by repeatedly evaluating JavaScript. Your
program supplies sample values, rather than instructions such as "play a piano
note." A short expression can therefore define an entire instrument or song.

The language is JavaScript running in the embedded QuickJS engine, not Rust and
not a separate bytebeat-specific language. This guide describes the behavior of
this native application; other bytebeat players can differ.

## Contents

- [Installation and building](#installation-and-building)
- [Getting started](#getting-started)
- [Editor, playback, and visualization](#editor-playback-and-visualization)
- [Audio buffering and pre-rendering](#audio-buffering-and-pre-rendering)
- [JACK playback on Linux](#jack-playback-on-linux)
- [NSM sessions](#nsm-sessions)
- [Time and sample rate](#time-and-sample-rate)
- [Playback modes and output values](#playback-modes-and-output-values)
- [Writing expressions](#writing-expressions)
- [Operators and mathematics](#operators-and-mathematics)
- [Stereo](#stereo)
- [State and initialization](#state-and-initialization)
- [Examples](#examples)
- [Saving scripts and metadata](#saving-scripts-and-metadata)
- [Upstream catalog and file loading](#upstream-catalog-and-file-loading)
- [Checking scripts from the terminal](#checking-scripts-from-the-terminal)
- [Errors, limits, and compatibility](#errors-limits-and-compatibility)
- [Adding a visualizer](#adding-a-visualizer)
- [Application icon](#application-icon)
- [Architecture and current scope](#architecture-and-current-scope)

## Installation and building

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
sudo apt install build-essential pkg-config libasound2-dev libjack-jackd2-dev libudev-dev \
  libx11-dev libxi-dev libxrandr-dev libxcursor-dev libxkbcommon-dev \
  libwayland-dev libgl1-mesa-dev
```

macOS requires the Xcode command-line tools. Windows requires the MSVC Rust
toolchain and Visual Studio C++ build tools.

### Release installers

Pushing a version tag runs `.github/workflows/release.yml`. After all builds
and installation checks succeed, the workflow publishes installers and
`SHA256SUMS` to a GitHub Release:

| Platform | Installer | Architecture |
| --- | --- | --- |
| Debian 12+ / compatible Ubuntu releases | `.deb` | x86-64 |
| Fedora 43+ | `.rpm` | x86-64 |
| macOS | `.dmg` containing a drag-to-Applications `.app` | Apple Silicon and Intel |
| Windows 10+ | Per-user `.exe` installer with an uninstaller | x86-64 |

For an on-demand build, open **Actions → Release installers → Run workflow**
and select a branch or tag. Manual runs use the version in `Cargo.toml`, run
the same build and installation checks, and upload installers as workflow
artifacts without creating or updating a GitHub Release. The workflow must
be present on the default branch for the **Run workflow** button to appear.

Tags must be `vVERSION` or `VERSION`, matching `[package].version` in
`Cargo.toml` exactly. SemVer prereleases such as `v0.2.0-rc.1` are supported
and published as prereleases; build metadata (`+...`) is not supported.
Commit any version changes, including `Cargo.lock`, before creating the tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

Linux packages include desktop entries, the SVG icon, and runtime dependencies.
Install a `.deb` with `sudo apt install ./bytebeat_*.deb`, or an `.rpm` with
`sudo dnf install ./bytebeat-*.rpm`. The optional upstream song catalog is not
bundled; built-in demos work offline and the catalog is downloaded on demand.

Windows installers are not code-signed, and macOS apps are ad-hoc signed rather
than Developer ID-signed or notarized, so OS security warnings are expected.
No signing secrets are required. Publishing uses the workflow's `GITHUB_TOKEN`
with `contents: write` permission only in the publishing job; builds have
read-only repository access. Rerunning a failed tag workflow replaces existing
assets for that tag.

## Getting started

1. Run `cargo run --release`.
2. Select the playback mode and sample rate specified by an example below.
3. Replace the editor contents with the JavaScript code.
4. Click **Apply & play**, or press **Ctrl+Enter** / **Cmd+Enter**.

Start at a low volume: arbitrary formulas can produce loud, harsh signals.
Edits are applied explicitly, not on every keystroke.

**Pause** preserves the running program. **Restart** creates a fresh program and
starts its time counter at zero, preserving whether playback was active.
**Stop** creates a fresh program at zero and leaves playback paused.
**Apply & play** compiles a fresh program and starts playback from zero.

Without `--catalog`, the application first looks for `./data/catalog.json`,
then `/usr/share/bytebeat/catalog.json` on Linux, then a previously downloaded
user-cache catalog. If none exists, it downloads the upstream library in the
background. The GUI remains usable during downloading and shows progress or a
retryable error in the song library. A completed cached library works offline
on subsequent launches. See [Upstream catalog and file loading](#upstream-catalog-and-file-loading)
for cache locations and manual download/refresh commands.

## Editor, playback, and visualization

- **Editor:** enter a JavaScript expression, then click **Apply & play** or
  press **Ctrl+Enter** / **Cmd+Enter**. Edits are applied explicitly, so incomplete
  typing does not interrupt playback. Scripts larger than **256 KiB** open in a
  read-only preview limited to **4 KiB**; playback and saving retain the complete
  source. To edit the full script, acknowledge the memory warning and click
  **Open full editor**. **Return to safe preview** restores protection. Consent
  resets when another song is loaded or the app restarts. Pasting more than
  256 KiB into a protected editor is blocked with a visible warning; enable full
  editing and paste again.
- **Playback:** Play/Pause, Stop (reset to zero), Restart, volume, sample-rate
  presets, and custom rates from 1,000 to 192,000 Hz. Playback starts paused at
  25% volume; use a comfortable listening level.
- **DC blocker:** optional checkbox beside Volume, on by default and saved with
  your settings. Removes DC offset using a first-order 10 Hz high-pass filter
  independently on the left and right channels, at the device's output sample
  rate, before volume adjustment and device conversion. It also attenuates very
  low bass; filter transients are clamped to the device's valid output range.
  Graphs still show the original, unfiltered signal. This reduces sustained DC,
  but is not a limiter or a guarantee against speaker damage: keep playback
  volume at a safe level.
- **Visualization:** open **Graphs** and use **+** to add a layer or **-** to
  remove it. Choose **Waveform**, **FFT spectrum**, **8-bit pattern**, or one of
  **35 original shader-art presets**. Each layer has a left/right/mixed channel,
  a color picker, and an opacity slider (0 is transparent; 255 is opaque).
  The Graphs menu stays open while you select and adjust layers.
  Layers overlay in list order; later layers draw
  over earlier ones. The defaults retain the stereo waveform and mixed spectrum.
  Use Play/Pause to control playback; clicking the display does not affect audio.
  Zoom adjusts the waveform and scrolling
  bit-pattern window; the spectrum always analyzes the latest 2,048 output frames
  using a Hann window. Graph settings, including an empty list, persist.
  The bit pattern shows eight horizontal bit rows (most significant at the top),
  with newer samples on the right. It maps normalized output to unsigned bytes
  (Bytebeat values are recovered exactly; other modes are quantized). All graphs
  use generated output before volume adjustment, without re-evaluating the song.

  Each graph's channel selection affects visualization only, not playback:

  | Setting | Signal displayed |
  | --- | --- |
  | **Left** | Left stereo channel only |
  | **Right** | Right stereo channel only |
  | **Mix** | Average of both channels: `(Left + Right) / 2` |

- **Shader art:** the preset selector includes blorp, seaqyuk, bhatsu, discinos,
  ?dot-line, morp, add-em-up, grid, circles, widr, fuzeball, undul, flwr, radonut,
  vu-w/max, notmizu, pulsedn, bebubebup, dncrs, discus, smutz, ball-o?3, incId,
  headrush, cubespace, s.o.i., volum, iblot, circlepower, c-pump, red ring, cybr,
  ball, qyube, and hexalicious. These are the original vertex shaders by
  **gman (Gregg Tavares)**, licensed **CC0** and bundled for offline use.
  Selecting a shader from a signal graph starts with white tint (original
  colors) and full opacity; use the color picker to tint it. Changing between
  shader presets preserves your tint and opacity. Shader layers use native
  OpenGL, depth-tested geometry, playback time, mouse position, and smoothed FFT
  and volume histories. Animation pauses with playback; histories reset when a
  song is compiled/restarted. Browser FFT/history timing may differ; touch input
  is neutral and no linked music is downloaded or played. Shader errors are
  logged and shown above the display. Multiple shader layers increase GPU cost.
  The menu links each artwork's source and attribution; licensing details are in
  [assets/VERTEXSHADERART-LICENSE.txt](assets/VERTEXSHADERART-LICENSE.txt).
- **Library:** search the included demos, load a song, or save your expression to
  **My songs**. Use the mode dropdown to show **All modes** (default), **Bytebeat**
  (unsigned), **Signed Bytebeat**, **Floatbeat**, or **Funcbeat**. The mode filter
  combines with text search and **My songs only**.
  Saving the same title replaces that local song; **x** deletes it.
  The editor, settings, and local songs persist through egui's native application
  storage when the application saves/exits. They are not written beside the binary.
  Search results are cached until the query, mode filter, saved-only filter, or
  library changes.

## Audio buffering and pre-rendering

The **Audio buffer** selector controls the queue between JavaScript rendering
and the output callback. Changes apply without recompiling the song or reopening
the audio device, and the selected size persists in application preferences and
NSM projects.

| Queue size | Approximate duration at 48000 Hz |
| --- | --- |
| 2048 frames (default) | 43 ms |
| 4096 frames | 85 ms |
| 8192 frames | 171 ms |
| 16384 frames | 341 ms |

A larger queue can absorb brief CPU/rendering spikes, but increases the delay
between generated and audible output. It cannot compensate for sustained
rendering slower than real time. New playback waits for the queue to fill before
starting output. Reducing its size lets already queued audio drain naturally.
This is not the operating system's device buffer; JACK's device rate and buffer
size remain controlled by the JACK server.

For expensive expressions, set **Pre-render** to **1 to 60 seconds** and click
**Prepare audio**. This compiles the current editor source from zero, pauses
playback, and generates the requested interval on the audio producer thread.
The progress bar shows preparation progress; the status bar shows prepared
audio still waiting to enter the output queue. Press **Play** when ready.
The duration setting defaults to 10 seconds and persists with the project.

Samples are generated sequentially, including all intermediate source samples,
so globals, closures, buffers, and other state evolve as during live playback.
Playing cached samples does not re-evaluate the program. Once the prepared
interval runs out, rendering continues live from the same program state.
**Cancel** stops preparation between render blocks and retains completed samples
and their corresponding state for playback.

Loading a song, **Apply & play**, **Stop**, **Restart**, or changing the playback
mode/sample rate discards prepared audio. Unapplied editor changes do not disturb
the running program; starting playback with unapplied changes recompiles instead
of playing the old cache. Volume, DC blocker, graphs, and queue-size changes do
not invalidate it. Prepared samples are held in memory, not saved with projects.

Pre-rendering allows a finite interval to play smoothly even if generating it
takes longer than its audio duration. It does not repair JavaScript errors or
distorted formulas, nor guarantee uninterrupted playback beyond that interval.
The existing JavaScript heap limit and 100 ms per-render-block execution budget
still apply; failures stop preparation and appear in the status bar and logs.
The PCM cache is outside the JavaScript heap limit: stereo float samples use
about 22 MiB for 60 seconds at 48000 Hz, plus bookkeeping. Memory use scales with
the output device's rate, not the song's formula rate.

## JACK playback on Linux

Use `--jack` to select JACK instead of the default audio backend:

```sh
cargo run --release -- --jack
./target/release/bytebeat --jack --file melody.js --memory-limit-mb 128
# Or use the installed executable:
bytebeat --jack --catalog /usr/share/bytebeat/catalog.json
```

A JACK server must already be running, or PipeWire's JACK compatibility must be
configured. Some PipeWire installations require `pw-jack` before the command.
If output is not automatically routed, connect the CPAL client to your playback
ports with a JACK/PipeWire patchbay.

JACK controls the output device rate and buffer size. The editor's sample-rate
control still determines the formula's source rate and the `sampleRate` passed
to JavaScript. The application's existing resampling converts between them.

JACK initialization errors are logged and shown in the status bar; the app does
not silently switch to ALSA or another backend. `--jack` is Linux-only and is
incompatible with `--check`, since checking does not open any audio device.

Linux builds include the JACK backend; no JACK feature flag is needed. Install
the JACK client library and development files (`libjack-jackd2-dev` on
Debian/Ubuntu). For RPM packaging, `pkgconfig(jack)` and `pkgconfig(alsa)` identify
the audio development dependencies; install the runtime JACK client library or
its PipeWire-compatible implementation as appropriate for the distribution.
The application does not start the JACK server itself. CPAL attempts to connect
output ports to system playback ports automatically; use QjackCtl or your
PipeWire patchbay if manual routing is needed.

## NSM sessions

Bytebeat supports [New Session Manager (NSM)](https://new-session-manager.jackaudio.org/)
and compatible managers. Add `bytebeat` as a client in the session manager;
the application detects `NSM_URL` automatically, with no extra command-line flag.
On Linux, managed sessions use JACK automatically. Start JACK (or configure
PipeWire's JACK compatibility) before opening the session.

The manager's **Save** command writes the complete editor project, including
source, title, playback settings, graphs, and "My songs", to `project.json` in
the assigned client directory. Reopening the session restores that project,
independently of standalone application preferences; new sessions start with
the default project. "Save to my songs" updates the session's song collection,
but does not replace the manager's Save command. Playback starts paused.

Managed JACK ports use a stable client-ID prefix, and automatic connection to
system playback ports is disabled so the session manager's patchbay can restore
routing. Use a connection-management client such as JACKPatch to save routing.
Closing a session terminates Bytebeat without a save dialog. Session switching
and optional-GUI/dirty-state extensions are not advertised: the manager must
restart the client when changing sessions.

Without `NSM_URL`, or when the manager cannot be contacted, startup remains
standalone. Headless `--check` ignores NSM entirely. UDP OSC URLs are supported;
unsupported URLs produce a warning and use standalone mode.

## Time and sample rate

The formula sample rate is the number of source samples generated per second.
It can be an integer or a fractional value between **1000 and 192000 Hz**.
This is separate from the output device's sample rate.

| Name | Bytebeat, Signed Bytebeat, Floatbeat | Funcbeat's returned function |
| --- | --- | --- |
| `t` | Source sample index: `0, 1, 2, ...` | Elapsed time in seconds |
| `sampleRate` | Selected formula sample rate in Hz | Selected formula sample rate in Hz |

For example, at **8000 Hz**, Bytebeat, Signed Bytebeat, and Floatbeat receive
`t = 0` through `7999` during the first second. At exactly one second,
`t = 8000`; the second second uses `8000` through `15999`. The counter increases
by one per source sample and does **not reset every second**.

At the same rate, Funcbeat's returned function receives `t` in seconds:
`0`, `0.000125`, `0.000250`, and so on, reaching `1` at one second.

For expression modes:

- Convert samples to seconds with `t / sampleRate`.
- Convert a duration in seconds to samples with `duration * sampleRate`.
- A sine at frequency `f` Hz has phase `2 * PI * f * t / sampleRate`.

For Funcbeat:

- `t` already measures seconds.
- A sine at frequency `f` Hz has phase `2 * PI * f * t`.
- The sample number can be reconstructed approximately as `t * sampleRate`.

The application supplies a new time value on every source evaluation. Assigning
to the expression's `t` parameter changes only that evaluation, not the playback
clock.

At lower source rates, samples are held until the next source value is due.
At rates above the output rate, some calculated samples are not output, but
every intermediate source sample is still evaluated. Stateful programs therefore
advance at the selected source rate.

## Playback modes and output values

### Bytebeat

Write an expression that produces an unsigned 8-bit value, conventionally
between `0` and `255`. Fractions are truncated toward zero and values wrap
modulo 256.

The actual normalized audio value is:

```text
wrapped = positive_modulo(truncate(value), 256)
audio = wrapped / 128 - 1
```

For example, `256` wraps to `0`, and `-1` wraps to `255`.
Value `128` maps to zero amplitude; `0` maps to `-1`, not silence.
The largest normalized positive value is `127 / 128`.

### Signed Bytebeat

Write an expression with signed 8-bit output, conventionally `-128` to `127`.
Fractions are truncated toward zero and values wrap as signed bytes:

```text
wrapped = positive_modulo(truncate(value) + 128, 256)
audio = wrapped / 128 - 1
```

Here `0` maps to zero amplitude, `127` maps to `127 / 128`, and both `128`
and `-128` map to `-1`.

### Floatbeat

Write an expression that produces a floating-point amplitude between `-1`
and `1`. Zero is silence. Finite values outside this range are clipped, not
wrapped.

Importantly, Floatbeat still receives `t` in **samples**, not seconds.

### Funcbeat

Write a JavaScript **function body**, including a `return` statement. This body
runs once during program initialization, with no arguments, and must return a
function. That function is then called for each source sample with:

```text
returnedFunction(timeInSeconds, sampleRate)
```

The returned function produces floating-point amplitudes, with the same
clipping rules as Floatbeat. Its parameter names are your choice: `(seconds,
rate)` works just as well as `(t, sampleRate)`. The factory can declare its own
local variables named `t` or `sampleRate`.

In all modes, output must be a number or a two-element array of numbers.
`NaN` holds the last valid normalized output independently for each channel,
or silence (`0` amplitude) if that channel has not yet produced a valid value.
Time and program state continue advancing, and valid output resumes normally.
A warning is logged only on the first `NaN` per compilation. Infinity remains
fatal. There is no automatic conversion of a scalar string, boolean, object,
or missing return value into an audio sample.

## Writing expressions

Bytebeat, Signed Bytebeat, and Floatbeat expect one expression. You do not
normally write `return` in these modes. A trailing semicolon is supported.

Useful expression forms include:

| Form | Meaning |
| --- | --- |
| `t * 3` | Arithmetic |
| `t & 255` | Bitwise operation |
| `t < sampleRate ? 128 : t & 255` | Conditional output |
| `[128, 160, 192, 224][(t >> 10) & 3]` | Array lookup |
| `(a = t * 3, b = t >> 8, a & b)` | Several operations; return the final result |
| `(() => { const x = t * 3; return x & 255; })()` | An immediately invoked function expression |

The comma operator evaluates left to right and returns its last operand.
Put parentheses around a comma expression when using it inside larger formulas
or as an array element.

For declarations, loops, or several statements, use an immediately invoked
function expression, or choose Funcbeat for a one-time setup section.
Declaring a local variable inside an immediately invoked function initializes
it again at every sample; it is not persistent oscillator or filter state.

JavaScript line comments (`// ...`) and block comments (`/* ... */`) are allowed.

## Operators and mathematics

### Built-in names and globals

The following names are available to your formulas:

| Name | Meaning |
| --- | --- |
| `t` | Source sample index starting at `0`; in Funcbeat's returned function, elapsed time in seconds |
| `sampleRate` | Selected formula sample rate in Hz, not necessarily the output device's rate |
| `sin`, `cos`, `floor`, `pow`, `random`, etc. | JavaScript `Math` functions without the `Math.` prefix |
| `PI`, `E`, etc. | JavaScript `Math` constants |
| `int(x)` | Alias for `Math.floor(x)` |
| `globalThis`, `window`, `self` | The same persistent global namespace, not a browser environment |

Technically, `t` is a function argument, not a global. Expression modes receive
`sampleRate` as an argument too, and `sampleRate` is also exposed globally,
including during Funcbeat initialization. In Funcbeat, declare the returned
function's arguments to receive time and rate.

For example, a 440 Hz sine in **Floatbeat**:

```js
0.25 * sin(2 * PI * 440 * t / sampleRate)
```

The equivalent in **Funcbeat**, where time is already in seconds:

```js
return (t, sampleRate) => 0.25 * sin(2 * PI * 440 * t);
```

### Common operators

| Operator | Meaning | Typical musical use |
| --- | --- | --- |
| `+`, `-`, `*`, `/` | Arithmetic; `/` is floating-point division | Mix signals, scale amplitude, change pitch |
| `%` | Remainder | Repeating ramps, beat positions |
| `**` | Exponentiation | Frequency ratios, envelopes |
| `&` | Bitwise AND | Mask values, select rhythmic bits |
| `\|` | Bitwise OR | Combine bit patterns |
| `^` | Bitwise XOR, **not exponentiation** | Combine or scramble patterns |
| `~` | Bitwise NOT | Invert bits |
| `<<`, `>>`, `>>>` | Left, signed-right, unsigned-right shifts | Time division and bit patterns |
| `condition ? a : b` | Conditional expression | Select notes or gate a voice |
| `&&`, `\|\|` | Short-circuit operations returning an operand | Conditional initialization or selection |

Most number-based bitwise operations convert operands to signed 32-bit
integers. `>>>` produces an unsigned 32-bit result. Shift counts use only their
lowest five bits: shifting by 32 is effectively shifting by zero.
Ordinary arithmetic uses JavaScript floating-point numbers.

JavaScript `%` is a remainder operation: `-1 % 256` is `-1`, not `255`.
For a positive cycle position, use `((x % period) + period) % period`.
The byte conversion at the output uses positive modulo separately.

Use parentheses liberally. In particular, arithmetic binds more tightly than
many bitwise operations: `(t * 3) & (t >> 8)` is easier to understand than
relying on operator precedence.

### Math aliases

You can use standard `Math` names without the `Math.` prefix, including:

| Category | Names |
| --- | --- |
| Constants | `PI`, `E`, `SQRT2`, `LN2` |
| Trigonometry | `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2` |
| Rounding | `floor`, `ceil`, `round`, `trunc` |
| Shaping | `abs`, `min`, `max`, `sign`, `sqrt`, `cbrt` |
| Exponents and logarithms | `pow`, `exp`, `log`, `log2`, `log10` |
| Randomness | `random()` |

`Math.sin(x)` and `sin(x)` are equivalent. Trigonometric inputs are in radians.
`random()` returns a value in `[0, 1)`; noise-based programs are not deterministic
across runs.

The additional alias `int(x)` means **`Math.floor(x)`**, not C-style truncation.
For example, `int(-0.5)` is `-1`, while `trunc(-0.5)` is zero. Use `trunc`
when truncation toward zero is required.

For musical pitch, multiplying frequency by `2` raises it one octave.
The ratio for `n` equal-tempered semitones is `2 ** (n / 12)`.

## Stereo

Return `[left, right]` instead of a single number. Each channel uses the selected
mode's output conversion independently. A scalar number is duplicated to both
channels.

Arrays must contain exactly two numeric values. An array of notes is valid as
an intermediate value, but select a note or turn it into audio before returning
it; the player does not interpret arbitrary arrays as a sequence.

The waveform displays the left channel in green and the right in blue.
The FFT analyzes the average of the two channels, so opposing-phase stereo
signals can cancel in the spectrum even when their individual waveforms are
visible.

## State and initialization

Funcbeat closures are the clearest way to preserve state between samples.
Place buffers, counters, filter history, and oscillator phase in the factory,
then update them in the returned function. See the final example below.

For example, this Funcbeat oscillator keeps its phase between samples:

```js
let phase = 0;
return (t, sampleRate) => {
  phase += 2 * PI * 220 / sampleRate;
  return 0.3 * sin(phase);
};
```

Expression modes can also store state on `globalThis`. Initialize it when
`t === 0`, then update it on subsequent evaluations. `window` and `self` are
aliases for `globalThis`; these names are available for compatibility, not as
browser environments.

Unqualified assignments such as `phase = 0` can create persistent globals in
non-strict code, as many catalog songs do. Prefer explicit `globalThis.phase`
or a Funcbeat closure for new programs.

A fresh compilation creates a fresh JavaScript runtime, discarding previous
globals and closures. Pausing does not reinitialize them.
Do not overwrite runtime helpers such as `sin` or `__bytebeat`.

## Examples

Each JavaScript block below is a complete program. Set its indicated mode and
sample rate before playing.

### 1. A simple sawtooth

**Mode:** Bytebeat. **Sample rate:** 8000 Hz.

```js
(t * 8) & 255
```

The masked ramp repeats every 32 samples: `8000 / 32 = 250 Hz`.
Try replacing `8` with `4` to lower it by one octave.

### 2. A stepped bitwise melody

**Mode:** Bytebeat. **Sample rate:** 8000 Hz.

```js
((t * ((t >> 12 & 3) + 2)) & 127) + ((t >> 4) & 63)
```

`t >> 12` advances slowly. Masking with `3` cycles through four pitch
multipliers, while the second term adds a slower stepped pattern.

### 3. A signed triangle wave

**Mode:** Signed Bytebeat. **Sample rate:** 8000 Hz.

```js
127 - abs(((t * 8) & 511) - 256) * 255 / 256
```

The ramp rises and falls rather than jumping straight from its maximum to
minimum. Folding a 512-step ramp and scaling it keeps the result between
`-128` and `127` before byte conversion. The cycle is 64 samples, or 125 Hz.

### 4. A clean sine wave

**Mode:** Floatbeat. **Sample rate:** 44100 Hz.

```js
0.25 * sin(2 * PI * 220 * t / sampleRate)
```

This is a 220 Hz sine at one-quarter amplitude. Because the formula divides
by `sampleRate`, changing the selected sample rate preserves the intended pitch.

### 5. A repeating four-note sequence

**Mode:** Floatbeat. **Sample rate:** 44100 Hz.

```js
(() => {
  const seconds = t / sampleRate;
  const note = [0, 4, 7, 12][floor(seconds * 2) % 4];
  const frequency = 220 * 2 ** (note / 12);
  const envelope = exp(-5 * ((seconds * 2) % 1));
  return 0.25 * envelope * sin(2 * PI * frequency * seconds);
})()
```

Two notes occur per second. The note array stores semitone offsets from 220 Hz,
and the exponential envelope decays within each half-second.
Changing frequency this way can introduce phase discontinuities; a phase
accumulator is better when smooth transitions are important.

### 6. A stereo pair

**Mode:** Floatbeat. **Sample rate:** 44100 Hz.

```js
[
  0.2 * sin(2 * PI * 220 * t / sampleRate),
  0.2 * sin(2 * PI * 330 * t / sampleRate)
]
```

The left channel plays 220 Hz and the right plays 330 Hz.

### 7. A decaying kick drum

**Mode:** Funcbeat. **Sample rate:** 44100 Hz.

```js
return seconds => {
  const beat = seconds % 0.5;
  const phase = 2 * PI * (45 * beat + 80 * (1 - exp(-25 * beat)) / 25);
  const envelope = exp(-12 * beat);
  return 0.4 * envelope * sin(phase);
};
```

The beat restarts twice per second. Its instantaneous frequency falls from
125 Hz toward 45 Hz, while its amplitude decays. No division by the beat
position is needed, so the first sample at time zero is valid.

### 8. A stateful oscillator and low-pass filter

**Mode:** Funcbeat. **Sample rate:** 44100 Hz.

```js
let phase = 0;
let filtered = 0;

return (seconds, rate) => {
  const note = [0, 7, 12, 4][floor(seconds * 2) % 4];
  const frequency = 110 * 2 ** (note / 12);
  phase = (phase + frequency / rate) % 1;

  const pulse = phase < 0.5 ? 1 : -1;
  const cutoff = 500;
  const coefficient = 1 - exp(-2 * PI * cutoff / rate);
  filtered += coefficient * (pulse - filtered);
  return 0.2 * filtered;
};
```

The phase and filter history are initialized once and persist between samples.
The phase accumulator changes pitch without resetting the oscillator's phase.
The filter smooths the square wave's sharp edges.

## Saving scripts and metadata

Save a program as a UTF-8 `.js` file, for example `melody.js`.
An optional `melody.json` sidecar supplies its playback settings:

```json
{
  "title": "My four-note melody",
  "author": "Your name",
  "mode": "Floatbeat",
  "rate": 44100
}
```

All four fields are required when a sidecar is present. Supported JSON mode
names are `"Bytebeat"`, `"Signed"` (or `"Signed Bytebeat"`), `"Floatbeat"`, and
`"Funcbeat"`. Rates may be fractional, such as `35498.66666`.

Without a sidecar, file loading defaults to Bytebeat at 8000 Hz.
The title is the filename without its extension. CLI settings override the
sidecar's mode and sample rate:

```sh
./target/release/bytebeat --file melody.js --mode floatbeat --sample-rate 44100
```

CLI mode names are lowercase: `bytebeat`, `signed`, `floatbeat`, and `funcbeat`.
`signed-bytebeat` is also accepted.
Opening a file loads the editor paused; click Play to start it.

Sources larger than **256 KiB (262144 bytes)** open in a read-only preview of at
most the first **4 KiB**, cut at a UTF-8 character boundary. The full source is
retained for playback, project persistence, and **Save to my songs**. Preview
text is never substituted for the actual program.

Full layout of very large scripts can consume gigabytes of GUI memory, which is
not covered by `--memory-limit-mb`. Check the warning acknowledgment and click
**Open full editor** to opt in for the current song. **Return to safe preview**
disables full editing again; loading another song or restarting also resets
consent. A paste larger than 256 KiB is blocked before layout unless full editing
has been enabled; the interface explains how to enable editing and paste again.

**Save to my songs** stores the current editor contents and settings in the
application's local library, not in a `.js` file. Saving an existing title
replaces that local song.

## Upstream catalog and file loading

Without `--catalog`, the application searches for an upstream catalog in this
order:

1. `./data/catalog.json`, relative to the current working directory.
2. `/usr/share/bytebeat/catalog.json` on Linux, for packaged installations.
3. `bytebeat/upstream/catalog.json` under the platform's user cache directory.
4. If none exists, download the upstream catalog and full song sources into
   that user cache.

The first automatic download runs in the background while the GUI stays usable
with its built-in demos. The song library shows progress, adds the upstream
songs when ready, and displays download errors with a **Retry download** button.
An incomplete download does not publish a cache index; verified source files
are reused on retry. Later launches use the completed cache without network
access. Supplying `--catalog` explicitly disables discovery and automatic
downloading; an invalid explicit path is reported rather than replaced.

Typical automatic cache locations are:

| Platform | Directory |
| --- | --- |
| Linux | `$XDG_CACHE_HOME/bytebeat/upstream/`, or `~/.cache/bytebeat/upstream/` |
| macOS | `~/Library/Caches/bytebeat/upstream/` |
| Windows | `%LOCALAPPDATA%\bytebeat\upstream\` |

The native downloader does not require Python or external commands. It retains
the upstream license, raw index, per-song metadata, and SHA-256 checksums.
Downloads use four workers, 30-second request timeouts, and up to three attempts
per request. Individual responses are limited to 32 MiB and the decompressed
catalog index to 64 MiB. No download is written into `/usr/share`.

To populate or refresh a repository/package's `data/` directory instead, the
Python 3 downloader remains available (standard library only):

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

The GUI adds the discovered or downloaded catalog to its built-in library.
Alternatively, specify a catalog or a song:

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
Without a local or installed copy, a first-time download needs network access.
If that download fails, the GUI still provides its built-in demos and local
compositions, alongside the visible download error.

Downloaded code is third-party JavaScript; the runtime has no filesystem/network
bindings, but its memory/time limits are not an operating-system security sandbox.
Keep upstream attribution and notices when sharing downloaded material.

## Checking scripts from the terminal

Check the first five seconds without opening a GUI or an audio device:

```sh
./target/release/bytebeat --file melody.js --mode floatbeat --sample-rate 44100 \
  --check --seconds 5 --report melody-report.json
```

Check a downloaded catalog and capture its console diagnostics:

```sh
./target/release/bytebeat --catalog data/catalog.json --check --seconds 1 \
  --report data/check-report.json > data/check.log 2>&1
```

`--seconds` accepts 0.01 to 60 seconds and defaults to 1.
The checker starts each program at time zero in a fresh runtime and uses
48000 Hz output. It continues after individual catalog failures.
Running `--check` without either `--file` or `--catalog` discovers a local,
installed, or cached catalog, or downloads upstream if none exists. Downloads
are synchronous in this headless mode and report progress in console logs.
Checking a particular `--file` never downloads a catalog. An explicit invalid
`--catalog` path is an error, not a trigger for automatic downloading.

With both `--file` and `--catalog`, check mode checks just the file; GUI mode
loads the file and the library. A catalog download failure is logged and exits
nonzero.

The checker uses the same engine and 128-frame render blocks as live playback.
The report records each song's mode/rate, failing stage, JavaScript diagnostic,
sample position in the error text, source samples checked in completed blocks,
peak normalized amplitude, thrown-text diagnostic count, and elapsed time.
GUI audio errors also appear in console logs. Set `RUST_LOG=bytebeat=debug`
for more detailed logging.

The JavaScript heap limit defaults to **32 MiB per song**. Buffer-heavy songs
can be given more memory with a positive integer `--memory-limit-mb` value:

```sh
./target/release/bytebeat --file melody.js --memory-limit-mb 128
./target/release/bytebeat --catalog data/catalog.json --check --seconds 1 \
  --memory-limit-mb 128 --report data/check-report.json
```

This option applies to both GUI playback and checking for the current invocation.
Units are MiB (1024 * 1024 bytes), despite the `mb` spelling. It limits the
JavaScript heap, not all application memory; raising it does not increase the
execution-time budget. Large limits can exhaust system memory, so use a value
appropriate for your machine.

The option applies to the current invocation and is not stored in project
settings.

Exit status is `0` when every selected program passes, or `1` for loading,
compilation, or rendering failures. Invalid command-line syntax can exit `2`.
Errors are logged to standard error with the filename and failing stage.
Rendering diagnostics include the source sample position and elapsed song time.
`RUST_LOG` controls logging; for example, `RUST_LOG=bytebeat=error` shows errors
without successful-check messages.

Text strings thrown during sample evaluation are console diagnostics, not fatal
errors. They are logged as warnings with their actual line breaks and source
sample position. For that sample, playback holds the previous normalized stereo
value, or silence before any valid output, and advances the source clock.
State changes made before the throw are retained. Every recovered throw is
counted in the report's per-song `thrown_texts` field, including for songs that
later fail. The report also records the selected `memory_limit_mb`.

All log messages escape terminal-control characters before writing to the
console. ESC is displayed as `\u{1b}`, carriage return as `\r`, and other C0/C1
controls (including tabs) as visible escapes; newlines and readable Unicode
remain intact. This prevents song diagnostics, exception messages, metadata,
and filenames from supplying terminal commands such as clipboard writes.
The escaping is a logging-only operation: JSON report values retain their
original text, thrown-text counts are unchanged, and audio behavior is unchanged.

### Rendering performance

Each successful result has a `performance` object; failed results use `null` so
partial checks cannot be mistaken for complete timing measurements:

| Field | Meaning |
| --- | --- |
| `output_frames` | Completed output frames, independent of the formula's source sample rate |
| `render_ms` | Sum of wall-clock time spent inside successful `Engine::render` calls |
| `realtime_factor` | `render_ms` divided by the output audio duration in milliseconds; less than 1 means faster than real time |
| `block_budget_ms` | Nominal 128-frame audio duration: approximately 2.67 ms at 48000 Hz |
| `block_p50_ms`, `block_p95_ms`, `block_p99_ms` | Nearest-rank percentiles of render-block latency |
| `block_max_ms` | Slowest render block |
| `blocks_measured` | Number of successful render blocks |
| `blocks_over_budget` | Blocks taking longer than their own output audio duration; the final partial block uses its actual frame count |
| `over_realtime_budget` | Whether average rendering cost exceeded the checked audio duration (`realtime_factor > 1`) |

The console prints these timings and warns when average rendering is too slow.
Warnings do not turn a compatible song into a failure or change the exit status.
For example, a factor of `0.25` means one second of audio needed about 250 ms of
rendering; `2.0` means it needed two seconds.

These measurements exclude file loading, compilation, peak calculation, and
report serialization. They include first-sample initialization and synchronous
diagnostic logging inside rendering. The existing `elapsed_ms` remains the
overall per-song check duration, including loading and compilation.

Use an optimized build and a sufficiently long interval when comparing results.
Short intervals can be dominated by initialization; timings depend on machine
load and power settings. A fast average does not rule out audible glitches from
individual spikes, device buffering, scheduling, GUI work, or other processes.
Larger buffers can absorb some spikes but cannot cure sustained overload.

A successful result verifies only the requested initial interval. It does not
prove that later sections work, that the audio sounds as intended, or that the
program can keep up with real-time playback. A passing song can have recovered
thrown-text diagnostics; inspect their count and logs when investigating it.

## Errors, limits, and compatibility

Compilation and fatal rendering errors are displayed in the status bar and
stop playback rather than being silently converted to audio.

| Symptom | Likely cause or action |
| --- | --- |
| Syntax error during compilation | Check parentheses, brackets, and the selected mode; a Funcbeat body needs to return a function |
| Non-numeric output | A missing `return`, an initialization expression ending in a string/object, or a wrong array lookup |
| NaN warning | Invalid math such as `0 / 0`; the affected channel holds its last valid output, initially silence |
| Infinity error | Division by zero or `log(0)`; playback stops |
| Wrong stereo shape | Return exactly `[left, right]`, not a note list or a one-element array |
| `... is not defined` | A missing initialization, misspelled variable, or unsupported host API |
| Out of memory | The program exceeded the JavaScript heap limit; default 32 MiB, adjustable with `--memory-limit-mb` |
| Interrupted | Initialization or a render block exceeded its 100 ms execution budget |
| Unsupported sample rate | The rate is non-finite or outside 1000 to 192000 Hz |
| Wrong pitch | Check the rate and whether `t` is samples or seconds in the chosen mode |
| Distorted Floatbeat | Reduce amplitude; finite output beyond `-1` to `1` clips |

Rendering begins at `t = 0`. Expressions such as `1 / t` and `log(t)` produce
infinity there unless guarded and stop playback. `sin(t) / t` produces `NaN`
at zero and initially outputs silence. Prefer a conditional to define the
intended time-zero result rather than relying on recovery.

Primitive text strings thrown by the sample function are logged and playback
continues as described above. JavaScript `Error` objects and non-string thrown
values remain fatal. A text throw from the Funcbeat factory is also fatal:
initialization must successfully return a playback function. Logging every
diagnostic can be expensive for songs that throw at very high rates; use
`RUST_LOG=bytebeat=error` to suppress warning output while retaining report counts.

Some upstream songs rely on their browser player tolerating NaN during
initial samples. These values are recoverable here too and do not fail CLI
checks. A passing check can therefore include held or silent output; inspect
the warning log when investigating formula bugs. Prolonged NaN output can
leave a constant signal and is not a substitute for fixing invalid calculations.

JavaScript arrays, objects, functions, closures, and typed arrays are available.
Browser and Node.js APIs are not: there is no DOM, `document`, `fetch`, timer
service, `console.log`, filesystem binding, or network binding. `window` and
`self` are only global namespace aliases.

Execution is on an audio producer thread. The 100 ms budget is a safety limit,
not a real-time performance target: even a program that stays within the limit
can be too slow to supply uninterrupted audio. Memory/time limits are not an
operating-system sandbox.

Other players can normalize bytes differently or coerce values this player
rejects. In particular, this application divides byte values by `128`, not
`127.5`; do not assume bit-for-bit equivalence with the web player.

## Adding a visualizer

Visualizers are built into the executable; there is no runtime import or plugin
loader. Signal graphs (waveform, FFT spectrum, and bit pattern) are implemented
in Rust in `src/visualizer.rs`. To add another signal graph, extend `GraphKind`,
its label, the Graphs menu, and the rendering match.

For shader art, no new Rust renderer is needed: append a preset to the JSON array
in `assets/vertexshaderart.json`. This uses the **VertexShaderArt-style GLSL
vertex shader** format, not Shadertoy fragment shaders or MilkDrop presets.

### Example: audio-reactive spectrum bars

This small shader draws 64 vertical lines. Each line's height follows a bin of
the selected channel's smoothed FFT spectrum:

```glsl
void main() {
    float bar = floor(vertexId / 2.0);
    float u = (bar + 0.5) / (vertexCount / 2.0);
    float level = texture2D(sound, vec2(u, 0.0)).r;
    float y = -0.8 + mod(vertexId, 2.0) * level * 1.6;
    gl_Position = vec4(u * 1.8 - 0.9, y, 0.0, 1.0);
    v_color = vec4(0.2, 0.8, 1.0, 1.0);
}
```

Append the following object **after the existing presets**, adding a comma
between objects. Existing saved layers reference preset indices, so keep the
existing order. JSON stores the GLSL code as a string; the example below is the
same shader on one line:

```json
{
  "id": "example-spectrum-bars",
  "name": "Example spectrum bars",
  "author": "Your name",
  "source": "https://example.com/your-visualizer",
  "license": "CC0-1.0",
  "settings": {
    "num": 128,
    "mode": "LINES",
    "backgroundColor": [0, 0, 0, 0],
    "shader": "void main() { float bar = floor(vertexId / 2.0); float u = (bar + 0.5) / (vertexCount / 2.0); float level = texture2D(sound, vec2(u, 0.0)).r; float y = -0.8 + mod(vertexId, 2.0) * level * 1.6; gl_Position = vec4(u * 1.8 - 0.9, y, 0.0, 1.0); v_color = vec4(0.2, 0.8, 1.0, 1.0); }"
  }
}
```

Replace the author and source with your own attribution. This example may be
used under CC0; use the appropriate license for any other shader you add.
`num` is the vertex count (two vertices per line here); supported drawing modes
are `POINTS`, `LINES`, and `TRIANGLES`. `backgroundColor` is normalized RGBA;
the transparent background lets earlier graph layers remain visible.

The renderer supplies `vertexId`, `vertexCount`, `time` (playback seconds),
`resolution`, `mouse`, and audio textures. `sound` contains normalized,
log-scaled FFT magnitudes: horizontal coordinates select frequency bins and
vertical coordinates select history, with the newest row at `0.0`.
`volume` provides peak/RMS and other audio metrics with history.
Write `gl_Position` and `v_color` in `main()`; do not add a `#version` directive
or redeclare the supplied inputs. The renderer adds the GLSL compatibility
wrapper and fragment shader.

Rebuild with `cargo run --release`, then open **Graphs**, add a layer with **+**,
and choose **Example spectrum bars** from its visualizer selector. Play a song
and select **Left**, **Right**, or **Mix** to choose its audio input. Shader
compilation errors appear above the display and in the logs.

When contributing additional presets, also update the catalog test in
`src/shader_art.rs`: it currently requires exactly 35 presets and checks that
every entry belongs to the original gman CC0 collection. Preserve those checks
for the original collection while adding coverage for new entries.

## Application icon

<img src="assets/bytebeat.svg" width="96" height="96" alt="Bytebeat Composer icon">

The scalable vector icon is [assets/bytebeat.svg](assets/bytebeat.svg). It uses
the GUI's waveform/spectrum colors and contains no fonts, raster images, scripts,
or external resources. The executable embeds this SVG and renders it at startup
for the native window/application icon, including the macOS Dock icon. No
external icon file is required to run the application.

For Linux/RPM packaging, install it at
`/usr/share/icons/hicolor/scalable/apps/bytebeat.svg` and use `Icon=bytebeat`
in the application's desktop entry. The SVG remains available unchanged for
desktop packaging. File-manager and platform bundle icons still require
platform-specific packaging.

## Architecture and current scope

`eframe`/egui draws the native GUI; `rquickjs` evaluates formulas; CPAL drives
the default output device; a bounded lock-free ring buffer separates JavaScript
from the real-time audio callback. `rustfft` supplies the spectrum. Formula
samples are converted to the device rate with zero-order hold (intentionally
preserving the classic stepped sound); rates above the device rate skip output
samples but still evaluate each source sample, preserving stateful formulas.
The scope and counter show audio delivered to the output queue, ahead of audible
output by up to the selected 2,048- to 16,384-frame queue plus the device buffer.
Pre-rendering advances program state separately without moving this display
ahead by the entire prepared interval.

This initial version has a plain monospaced editor and the core
compose/play/visualize workflow, with an optional downloaded upstream library.
It does not import share URLs, export recordings, or include all of the web app's
editor themes and visualization modes. No upstream application code is bundled.

### Development checks

```sh
cargo test
cargo clippy --all-targets -- -D warnings
python3 -m unittest discover -s scripts -p 'test_*.py'
```
