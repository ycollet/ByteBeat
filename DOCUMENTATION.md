# Bytebeat language guide

Bytebeat Composer generates sound by repeatedly evaluating JavaScript. Your
program supplies sample values, rather than instructions such as "play a piano
note." A short expression can therefore define an entire instrument or song.

The language is JavaScript running in the embedded QuickJS engine, not Rust and
not a separate bytebeat-specific language. This guide describes the behavior of
this native application; other bytebeat players can differ.

## Contents

- [Getting started](#getting-started)
- [JACK playback on Linux](#jack-playback-on-linux)
- [Time and sample rate](#time-and-sample-rate)
- [Playback modes and output values](#playback-modes-and-output-values)
- [Writing expressions](#writing-expressions)
- [Operators and mathematics](#operators-and-mathematics)
- [Stereo](#stereo)
- [State and initialization](#state-and-initialization)
- [Examples](#examples)
- [Saving scripts and metadata](#saving-scripts-and-metadata)
- [Checking scripts from the terminal](#checking-scripts-from-the-terminal)
- [Errors, limits, and compatibility](#errors-limits-and-compatibility)

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
on subsequent launches. See the [README](README.md#upstream-catalog-and-file-loading)
for cache locations and manual download/refresh commands.

## JACK playback on Linux

Use `--jack` to select JACK instead of the default audio backend:

```sh
cargo run --release -- --jack
./target/release/bytebeat --jack --file melody.js --memory-limit-mb 128
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
See the [README's JACK setup notes](README.md#jack-audio-on-linux) for build and
packaging dependencies.

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
