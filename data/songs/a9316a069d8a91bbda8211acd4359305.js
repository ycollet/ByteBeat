/*!
 * •───────────────•°•❀•°•───────────────•
 * PrincessPriscilaPT · きらめき☆Skyblossom
 * •───────────────•°•❀•°•───────────────•
 */

	/* filters */
t || (fx = [], sfx = []), fxi = 0, sfxi = 0,
lpf = (inp, cut) => (
	lpf_fxii = fxi++,
	fx[lpf_fxii] ??= 0,
	fx[lpf_fxii] += (inp - fx[lpf_fxii]) * cut
),
hpf = (inp, cut) => inp - lpf(inp, cut),
bpf = (inp, hcut, lcut) => hpf(lpf(inp, lcut), hcut),
bbf = (inp, hcut, lcut, boost) => inp + bpf(inp, hcut, lcut) * boost,

acc = a => (
	acc_fxii = fxi++,
	fx[acc_fxii] ||= 0,
	fx[acc_fxii] += a / max(1, t)
),

sli = (a, n) => (
	sli_fxii = sfxi++,
	sl = sfx[sli_fxii] ??= {x: 0, y: 0},
	isNaN(a) ? 0 : (sl.x += sl.y += (a / max(1, t) - sl.y) / n)
),

this.buf ??= {}, buf = this.buf,
cosp = (a, b, m) => {
	mu = (1 - cos(m * pi)) * .5;
	return a * (1 - mu) + b * mu;
},
wsin = (phase) => (-cos(phase / 256 * pi) + 1) * 128 - .5,
dMax = 1e5,
	/* delay/reverb */
dly = multiTapDelay = (inp, heads, mix, fbfn = x => x) => {
	dly_fxii = dlyfxi++;
	buf[dly_fxii] ??= new Float32Array(dMax);
	dly_wi = dt % dMax;
	dly_feed = inp;
	dly_out = 0;

	for(let head of heads){
		pos = dMax + dt - head.t;
		i0 = floor(pos) % dMax;
		i1 = (i0 + 1) % dMax;
		mu = pos - floor(pos);

		dly_ri = cosp(
			buf[dly_fxii][i0],
			buf[dly_fxii][i1],
			mu
		);

		dly_feed += dly_ri * head.fb;
		dly_out += dly_ri * head.m;
	};

	buf[dly_fxii][dly_wi] = fbfn(dly_feed);

	return inp * (1 - mix) + dly_out * mix;
},

dlyfxi = 0,
this.delayClock ??= 0,
dt = this.delayClock++,
pi = PI, rnd = random,

	/* track settings */
bpm = 109,
sr = 48e3,
spb = (60 / bpm) * sr,
sec = t / sr,
ts = sec * (bpm / 60) * 16384,
tt = sec * 256 * 440 * 2 ** (-5 / 12),
gain = (dB = 0) => 10 ** (dB / 20),

rt = this.delayClock ?? 0,
	/* reverb heads */
rvrbHeads = [[
	{t: 600 + sin(rt / 8e3) * 40, m: 1, fb: 0},
	{t: 5e3 + wsin(rt / 180), m: .6, fb: .3},
	{t: 1e4 + wsin(rt / 300), m: .5, fb: .5},
	{t: 19e3 + wsin(rt / 380), m: .3, fb: .7},
	{t: 35e3 + wsin(rt / 420), m: .2, fb: .9},
], [
	{t: 600 + cos(rt / 8e3) * 40, m: 1, fb: 0},
	{t: 11e2 + wsin(rt / 200), m: .6, fb: .3},
	{t: 13e3 + wsin(rt / 320), m: .5, fb: .5},
	{t: 14e3 + wsin(rt / 320), m: .3, fb: .7},
	{t: 4e4 + wsin(rt / 450), m: .2, fb: .9},
]],
	/* chorus heads */
chrsHeads = [[
	{t: 9e2 + sin(rt / 4e3) * 50, m: 1, fb: .2},
	{t: 17e3 + wsin(rt / 310), m: .5, fb: .65},
	{t: spb / 4.05 * 3 + cos(rt / 35e3) * 175, m: 1,  fb: .75}
], [
	{t: 9e2 + cos(rt / 4e3) * 50, m: 1, fb: .2},
	{t: 16e3 + wsin(rt / 340), m: .5, fb: .65},
	{t: spb / 3.95 * 3 + sin(rt / 35e3) * 175, m: 1, fb: .75}
]],

	/* sequencers */
pI = x => parseInt(x || "_", 36),
seq = x => acc(tt * 2 ** (pI(x) / 12)),
sql = (x, n) => sli(tt * 2 ** (pI(x) / 12), 512),
chr = ["9cgh", "behj", "begj", "9cgj", "9ceh", "7beh", "7bcg", ts >> 19 & 1 ? "7acg" : "7bcg", "9cgh", "behj", ts >> 19 & 1 ? "8egk" : "begj", "9cgj", "9ceh", "7beh", "7bcg", ts >> 19 & 1 ? "7bcg" : "9cgj"][((ts >> 12 & 15) > 6) + (ts >> 16) * 2 & 15],
mel = "5 c   eee b c b 7 e   ggg e c bc5 c   eee b c b 7 e   ggg h g c"[ts >> 12 & 63],
mel2 = "555 b cc  bb  ce  e bb  bcc b c 777 b cc  ee  eg  g bb  ccc b c 555 b cc  bb  ce  e bb  bcc b c 777 c hj  hh  eg  g hh  ggg c 7 555 b cc  bb  ce  e bb  bcc b c 888 b cc  hj  eg  g bb  ccc b c 555 b cc  bb  ce  e bb  bcc b c 777 c hj  hh  eg  g hh  ggg c 7"[ts >> 11 & 255],
lead = "c 9j c ee 7 b c e je b cc b 9 b c 5h g ee c b c b 7g b cc 7 c e"[ts >> 12 & 63],
bas = "557744992277cc99557744992277cc99557744992277cb97557748942277cccc"[ts >> 14 & 63],

	/* song progression */
prog = (str = "", spd = 0) => {if (str === " ") return 0; return str[ts >> spd] | 0},
kProg = prog("0111001100", 19), /*   kick    */
sProg = prog("0011011100", 19), /*   snare   */
hProg = prog("0111111110", 19), /*    hat    */
mProg = prog("1111011111", 19), /*   melody  */
lProg = prog("0011001111", 19), /*   lead    */
cProg = prog("1111111111", 19), /*  chords   */
rProg = prog("1101010101", 19), /*   riser   */
bProg = prog("1111111111", 19), /*   bass    */

	/* vibrato */
vi = (sin(ts / 640) / 2 + sin(ts / 2101) * 2 + sin(ts / 1234)) * 16,

	/* waveforms */
fmw = (phase, index, mod, fb, mix) => sin(pi / 128 * phase + index * sin(mod * pi / 128 * phase + fb * sin(mod * pi / 128 * phase)) * mix),

wvm = t => fmw(t / 2, 1, 3, 0, .55) + fmw((t + vi) * 1.005, 2, 3, 0, .15) / 2 || 0,
wvl = t => fmw(t / 2, 2, 2, 1, .75) + fmw(t, 2, 2, 1, .35) || 0,
wvc = t => (cos(sqrt(t / 2 % 256) * pi / 8) / 1.45 + bpf((t + vi) / 256.85 % 2 - 1 + t / 254.65 % 2 - 1 + (t + vi) / 257.95 % 2 - 1, .1, .1) * 1.85) / 1.25 || 0,
wvb = t => bpf(tanh((sin(sin((t + vi / 2) * pi / 63.65 * .75) / 4 + sin(t * pi / 128) / 2 + sin(t * pi / 257.35)) + sin(t * pi / 256))) + sin(t * pi / 512) / 12, .01, .1) * 1.65 || 0,

body = sin(sin(110 / 4 ** (ts % 8192 / 3e3))),
noise = () => bpf(bpf(rnd(), .3, .4), .3, .4) * 8 * cbrt(~ts / 8192 % 1 + .125),

	/* drums */
kick = bbf(sin(sin(4 * (ts % 16384) ** .35)) * (1 - min(1, ts / 4096 % 4) ** 4) / 2.1, .025, .05, -.8),
snare = () => bbf(hpf(tanh((body + noise()) * (1 - ts / 8192 % 1) * sqrt(2)) * (1 - ts / 8192 % 1), .005), .01, .2, -1) *! (ts >> 13 & 1) * (ts >> 14 & 1) / 2.5,
hat = () => (bpf(rnd(), .3, .45) * .9995 ** (ts % 8192) *! (ts >> 13 & 1) * 1.3 + bpf(rnd(), .45, .7) * .9991 ** (ts % 4096) * (ts >> 13 & 1) * 1.3) / 2.4 * (1 ? min(1, ts / 16384 % 1 * 3) ** 3 * .65 + .35 : 1),
riser = () => bpf(rnd(), .3, .35) * (ts / 2 ** 19 % 1) ** 32 * rProg * 1.55,
brk = rProg ? !((ts >> 16 & 7) == 7) : 1,

	/* mix and mastering */
lr = lr => bpf(tanh(lpf(dly((wvc(seq(chr[0])) + wvc(seq(chr[1])) + wvc(seq(chr[2])) * min(1, ts / 2 ** 18) ** 2 + wvc(seq(chr[3])) * min(1, ts / 2 ** 19) ** 2) / 3.65 * cProg + dly((wvm(seq(ts >> 19 & 1 ? mel2 : mel) * 2) * mProg * 1.15 + wvl(sql(lead) * 2) * lProg / 2.15) * min(1, ts / 2 ** 19) ** 2 * 1.25, chrsHeads[lr], .45, x => tanh(bpf(x, .02, .5) / 180) * 100), rvrbHeads[lr], .35, x => tanh(bpf(x, .01, .8) / 180) * 100) * min(1, ts / 2 ** 19) ** .5 / 1.85 + wvb(seq(bas) / 4) * min(1, ts / 2 ** 19) ** 3 / 2.15 * bProg, min(1, ts / 2 ** 19) ** 1.5) * (kProg ? min(1, ts / 16384 % 1 * 2.5) ** 2 * .95 + .05 : 1) + bpf((kick * kProg + snare() * sProg + hat() * hProg) * 2.8 * brk + riser(), .005, .65)), .0005, .8),

	/* output */
[lr(0), lr(1)]