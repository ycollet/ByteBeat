/*!
 *      •───────────────•°•❀•°•───────────────•
 * PrincessPriscillaPT · Noontide Café in Maretime Bay
 *      •───────────────•°•❀•°•───────────────•
 */

t || (fx = {}), fxi = 0, T = t,

	/* Clipping */
clip = (signal, mode = 0) => mode ? min(1, abs(signal)) * sign(signal) : tanh(signal),

	/* High-pass */
hpf = (signal, id = "hpf", cut = .0025) => (
	fx[id] ??= 0,
	signal - (fx[id] += (signal - fx[id]) * cut)
),

	/* Delay Line */
dlay = (signal, length, feedback) => (
	length = round(length),
	buffer = fx[++fxi] ??= Array(length).fill(0),
	output = signal + buffer[T % length],
	buffer[T % length] = output * feedback,
	output
),

	/* Additive mixing */
mix = (...channels) => channels.reduce((inst_x, inst_y) => inst_x + inst_y, 0),

	/* Delta PCM simulation */
presets = {
	default: {rate: 33143.94, steps: 32},
	portable: {rate: 48000, steps: 24},
	retropocket: {rate: 22050, steps: 16},
	nes: {rate: 12604, steps: 32}
},

dpcm = (signal, mix = 1, preset = "default", id = "default") => {
	let config;

	if (typeof preset === "string") {config = presets[preset];} else {config = preset;};

	if (!config) {throw new ReferenceError(`Unknown DPCM preset: ${preset}`);};

	// Create state
	let state = fx[id];

	if (!state) {state = fx[id] = {output: 0, target: 0, frame: -1, gate: 0};};

	// Cache preset values
	const sampleRate = config.rate;
	const stepCount = max(1, trunc(config.steps));
	const stepSize = 1 / stepCount;

	// Clamp mix
	mix = max(0, min(1, mix));

	// Current sample frame
	const frame = (t * sampleRate / sr) | 0;

	// Input gate
	state.gate = abs(signal) > 0 ? 1 : 0;

	// Update once per DPCM sample
	if (frame !== state.frame) {

		state.frame = frame;

		const input = max(-1, min(1, signal));

		if (input > state.target) {
			state.target += stepSize;
		} else {
			state.target -= stepSize;
		}
	}

	// Slew toward target
	state.output += state.target - state.output;

	// Reflect at limits
	if (state.output > 1) {
		state.output = 1 - state.output;
	}

	if (state.output < -1) {
		state.output = -1 - state.output;
	}

	// Dry/wet mix
	return state.output * state.gate * mix + signal * (1 - mix);
},

	/* chords */
chrd = (tone = t, note = [0], trnsp = 0, wave = input => input % 2 - 1) => {
   let chr = 0; let poly = 4;

   if (typeof note == "string")
      note = [...note].map(x => parseInt(x, 36));

   for (let idx = 0; idx < min(poly, note.length); idx++)
      chr += wave(tone * 2 ** ((note[idx] + trnsp) / 12));

   return poly ? chr / poly : 0;
},

	/* Track Settings */
bpm = 140,
sr = 48e3,
spb = (60 / bpm) * sr,
sec = t / sr,
ts = sec * (bpm / 60) * 16384,
swg = 1, ts -= (ts / 4 & 2048) * swg, /* SWANG */
tt = sec * 440 * 256 * 2 ** (-6 / 12) / 2,
gain = (dB = 0) => 10 ** (dB / 20),

	/* Sequencers */
not = (string = "", index) => 2 ** (parseInt(string[index], 36) / 12),
seq = (string = "", index) => parseInt(string[index], 36),
rhm = max(((0xb1b6edbb >> (ts >> 12 & 30) & 3) - ts / 8192 % 1) / 4, 0),
rhb = max(((0xbefbdbdb >> (ts >> 12 & 30) & 3) - ts / 8192 % 1) / 4, 0),
snc = ((ts >> 13 & 15) > 6) + (ts >> 16 | 1) - 1,
grc = (string, index, speed) => 2 ** (parseInt(string[index] - string[index] *!! speed, 36) / 12),

	/* Song progression */
prog = (str = "", spd = 0) => {if (str === " ") return 0; return str[ts >> spd] | 0},
kProg = prog("011111011100", 19), /*   kick    */
rProg = prog("011111011110", 19), /*    rim    */
nProg = prog("111111111110", 19), /*   noise   */
aProg = prog("001111001110", 19), /*    arp    */
mProg = prog("001111001111", 19), /*   melody  */
lProg = prog("000000111111", 19), /*   lead    */
cProg = prog("111111111111", 19), /*  chords   */
bProg = prog("111111111111", 19), /*   bass    */

	/* Melodics */
mel = tt * not("      9 ggeec eeee  ggee  7 bbc ee  ggee  cc  bb  ccbb99  7 99b cc  5 55  55  cc  bb7 77  55  55  443 4444444444    4444555577  99  9 99  99  gg  eec ee  7 bbc ee  jjee  ccb bb  ccbb99  bbc gg  hhggeeeeeeee            ccb cccccccccccccccccc", ts >> 12 & 255) / grc("00002000002000200000000200000000100000010000000000000000000000002000000200000000200000020000000200000000000000010000000000000", ts >> 13 & 127, ts >> 11 & 3),
mel2 = tt * not("c 9j c ee 7 b c e je b cc b 9 b c 5h g ee c b c b 7g b cc 7 c e", ts >> 13 & 63) / grc("0002000200000000002000010000000010000002000000000000000100001000", ts >> 13 & 63, ts >> 11 & 3),
bas = tt * not("554927cc55492500", snc & 15) * not("00    7 77    0 00    7 77  00", ts >> 12 & 31),
chrN = [
	"59cg", "58ce", "47be", "479c",
	"259c", "257b", "047b", "047a",
	"59cg", "58ce", "47be", "479d",
	"259c", "058c", "047b", "047b"
][((ts >> 17 & 7) > 6 ? 14 : snc & 15)],
arp = tt * not(chrN, ts >> 10 & 3),

	/* Waveforms */
/* 2A03 */
pul = (phase, id = "pul", width = 25.0) => hpf((phase / 256 % 1 < width / 100 ? 1 : -1) - (width / 50 - 1), id) / 2 || 0,
tri = (phase, id = "tri") => hpf(asin(sin((phase | 7) * PI / 128)) / PI, id) || 0,
noi = (level, id = "noi") => (
	hpf(sign(T % 2 ** (6 - (level % 7)) ? fx[++fxi] : fx[++fxi] = random() - random()) / 2, id) || 0
),
dmc = (signal, id = "dpcm") => hpf(dpcm(signal, 1, "default", id), id + " "),

/* VRC6 */
saw = (phase, id = "saw") => hpf(round((phase / 128 % 2 - 1 || 0) * 3.5) / 4, id),

/* OPLL */
fmw = (phase, level) => sin((fT = ((phase - 7) | 7) * PI / 128) + sin(fT * 1.005 + cos(fT * 2)) * level) * 1.2,

	/* Drums */
kik = tanh(sin(sqrt(ts % 8192)) * .9995 ** (ts % 8192)) * (1 - min(1, ts / 8192 % 1) ** 4) / 1.3,
rim = sin(t / 13 * 2 + sin(t / 5 * 2)) * .998 ** (ts % 8192) * 4,
drm = kik * seq("1001100100001000", ts >> 13 & 15) * kProg + rim * seq("0010010010010010", ts >> 13 & 15) * rProg,

	/* Channels */
/* 2A03 */
ch_00 = lr => dlay((pul(mel, "pul", 15)) / 3.5 * mProg, spb * 3 / 4 * (lr ? .995 : 1.005), .3),
ch_01 = lr => pul(arp * (lr ? 1.005 : .995), "pul2") * rhm / 3.5 * aProg,
ch_02 = tri(bas / 4) * rhb ** 2 * 2 * bProg,
ch_03 = () => noi(seq("1535535535535535", ts >> 13 & 15)) * (1 - ts / 4096 % 1) *! (ts >> 12 & 1) / 4.5 * nProg,
ch_04 = dmc(drm, "dpcm"),

/* VRC6 */
ch_05 = lr => dlay((pul(mel * 2 * (lr ? .995 : 1.005), "pul2", 40)) / 7 * mProg, spb / 2, .45),
ch_06 = lr => dlay((pul(mel2 * 4 * (lr ? .997 : 1.003), "pul3", 50)) / 10 * lProg, spb * 3 / 4, .55),
ch_07 = lr => saw(bas / 2 * (lr ? .995 : 1.005)) * rhb ** 2 / 2 * bProg,

/* OPLL */
ch_08 = lr => chrd(tt, chrN, 0, inp => hpf(fmw(inp * (lr ? .995 : 1.005), rhm ** 2 + .5, 1.005), "chrd", .002) * (rhm ** 1.55 + .05)) * cProg,

	/* Output */
out = lr => hpf(round(clip(mix(
	ch_00(lr),
	ch_01(lr),
	ch_02,
	ch_03(lr),
	ch_04,
	ch_05(lr),
	ch_06(lr),
	ch_07(lr),
	ch_08(lr)
) * gain(-.65), 0) * 48) / 48, "mast", .0002),

[out(0), out(1)]