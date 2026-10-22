class RS {
    constructor() {
        this.phase1 = 0;
        this.phase2 = 0;
        this.env1 = 0;
        this.env2 = 0;
        this.aOut = 0;
        this.aOutPrev = 0;
        this.aInPrev = 0;
    }

    trigger() {
        this.env1 = 1;
        this.env2 = 1;
        this.phase1 = 0;
        this.phase2 = 0;
    }

    process(sampleRate) {
        this.outPrev = this.out;

        this.phase1 += 455 / sampleRate;
        this.phase2 += 1667 / sampleRate;
        if (this.phase1 > 1) this.phase1 -= 1;
        if (this.phase2 > 1) this.phase2 -= 1;

        this.env1 *= Math.exp(-1000 / (3 * sampleRate));
        this.env2 *= Math.exp(-1000 / (2 * sampleRate));

        const osc1 = Math.sin(this.phase1 * 2 * Math.PI) * this.env1;
        const osc2 = Math.sin(this.phase2 * 2 * Math.PI) * this.env2;

        let oscmix = (osc1 + osc2);
        
        const drive = 12;
        const bias = -0.43;
        
        // Simple hard clip distortion
        let aIn = oscmix * drive + bias;
        aIn = Math.min(Math.max(aIn, -1), 1);
        
        // DC blocker
        const R = 0.997;
        this.aOut = aIn - this.aInPrev + R * this.aOutPrev;
        this.aInPrev = aIn;
        this.aOutPrev = this.aOut;

        return this.aOut;
    }
}

const rs = new RS();

return function (time, sampleRate) {
	const t = Math.round(time * sampleRate);
    const currentSecond = Math.floor(time);
    if (t % sampleRate == 0) {
        rs.trigger();
        lastTime = currentSecond;
    }

    return rs.process(sampleRate);
}