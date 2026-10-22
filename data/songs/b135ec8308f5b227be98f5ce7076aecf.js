fmbass=x=>sin(x*PI/128+sin(x*PI/256)+abs(cos(x*PI/64))),
fmwave=x=>abs(sin(x*PI/32)+cos(x*PI/128)+cbrt(sin(x*PI/64))+abs(sin(x*PI/16))/2)-1,
63*(fmbass((t*'36364689'[t>>13&7]/12))*((1-t/8192%1)**.5)+(fmwave((((t>>12^(t>>12)-2)%11*t/2)))*((1-t/8192%1)**.5)/2))