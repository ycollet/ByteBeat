t=int(t/4),
l=sin(t>>3),
h=sin(t),
proc=(input,s=11)=>(x=input[(t>>s)%input.length],(x=='h'?h:x=='l'?l:x)*t&t>>(s-8)),
proc("l2h2l2h2l2h2l2h3",12)-8