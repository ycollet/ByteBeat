BaseFrequency = 440,
Tone = ((2 ** (-9 / 12)) * BaseFrequency * 128) / 48e3,
Note = (Input, Transpose=0) => (t * Tone * 2 ** ((Input - Transpose - 3) / 12)),
Speed = t * .8,

Melody = Speed => Speed > 0 && ((Note([31, 24, 27, 31, 26, 19,22, 26, 27, 20, 24, 27, 29, 22, 26, 29][15 & Speed >> 13]) & 128) % 256 - 64) * max(.5, 1 - Speed / 8192 % 1),

Melody(Speed) + Melody(Speed - 12288) / 2 + Melody(Speed - 24576) / 3 + Melody(Speed - 38168) / 4