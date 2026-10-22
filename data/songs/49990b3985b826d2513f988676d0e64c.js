BPM = 140, m = "map",

SR = 48e3,

T = t * BPM / 60 * SR / 131072,

t ? 0 : b = mem = Array(3e6).fill(0),

bar = 14,

n = $ => 2 ** ($ / 12) * 2 ** (3 / 12),

L = [[0, 3, 7, 10], [0, 3, 5, 7, 12], [1, 5, 8, 12], [0, 2, 5, 7, 12]][(T / (4 / 1)) >> bar & 3],

chrd=t=>L[m](chrd => (t * n(chrd)) % 256 / L.length).reduce((a, b) => a + b) / 2,

p = ($, S, s) => n(parseInt($[((T / (S)) >> s) % $.length], 36)),

bt = w => ((w & 255) / 128) - 1,

dec = (s, r, S) => Math.max((1-T/s%1)**r,S),

sch = (s, r, S) => Math.max((T/s%1)**r,S),

lp = (i, f) => (
$$ = 0, $ = $$++, mem[$] ??= 0, mem[$] += ((i) - mem[$]) * f
),

hp = (i, f) => (i - lp(i, f)),

lpr = (i, f, R) => ($$ = 0, $ = $$++, 
t || (b[$+'lp6'] = 0, b[$+'lp12'] = 0),
ct = Math.min(f, .999),
X = R + R / (1 - ct),
b[$+'lp6'] += ct * (i - b[$+'lp6'] + X * (b[$+'lp6'] - b[$+'lp12'])),
b[$+'lp12'] += ct * (b[$+'lp6'] - b[$+'lp12'])),


//Zynx92's TB303 thingy (Modified)

s = t => t % 256 / 256 - .5,
S = t => sin(t * PI / 128),
TB = (m, x) => lpr(Math.max(-2, Math.min(2, (s(t * m) + (S(t % (256 / m) * x) * (s(t * m) - .5))) * 5))/2, .4, .5),

tb = TB(p("777333AC", 1/4, bar) / 4, Math.sin(T / 4 / 48e3) * 4) / 2 * dec(4096, 2, .5),

c = (bt(chrd(t))),

ch = (tb + (c + .5) / 1.33) * (SS = ([S = sch(2048, 3, .1), 1, 1, 1, 1, 1, 1, 1][T >> 11 & 7])),

k = sin(1.1 * sin(8 * cbrt(T % 16384))) * dec(16384, 5, .1),

h = hp(2 * random() - 1, .7) * dec(4096, 3, .01),

k + ch + h * SS