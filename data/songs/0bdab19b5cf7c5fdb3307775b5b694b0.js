data=`\x80ÀàðúðàÀ\x80@ \x10\x06\x10 @\x80ÀàðàÀ\x80@ \x10 @\x80ÿ\x80\x01`,
wave=x=>data.charCodeAt((int(x/4))%data.length),
t1=t/4,h=t/4*((t&4096?t:-t)>>9&7)*(t<1e4),
wave(h)