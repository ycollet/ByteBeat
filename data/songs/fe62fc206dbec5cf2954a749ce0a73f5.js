t?0:a=0, // remove error i guess
b=(t*2**(-"0042"[t*PI>>16&3]/12)>>5)%4, // melody
((sin((d=t*PI)/b&4095)<0)-(t/b&1&&1)+1)?a-=a/2+b*64:a=128, // floating point shenanigans
((d&12288?abs(a/2)*((d>>12)%3+1)%128:sin(cbrt(d%16384)*6)>0&&(1-d/4096%1)**1.5*128))*2 // the song