dist=128,
t||(trg=0,l=0,c=0),
(t%dist)||(l=trg,trg=random()-0.5),
lerp=(v0,v1,t)=>v0*(1-t)+v1*t,
c=lerp(l,trg,t%dist/dist)