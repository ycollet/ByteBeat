a='0  01   0   1   0   1   0  01   0   1   0   1   0   1   0 001 0 ',
b='2221111111111111333322221111444444111111111111113333555555111111',
drums=lr=>((pat1=a[(t>>13)%a.length],pat1=='0'?(tan(cos((5+lr)*cbrt(t%8192))))*(-t>>5&255)/80:pat1=='1'?(random()+lr)*(-t>>5&255)/80+(tan(cos((15+lr)*cbrt(t%8192))))*(-t>>5&255)/80:0)*((-t>>6&255)^800)/1500)*3,
bass=lr=>(pat2=b[(t>>13)%b.length],pat2=='1'?sin(cbrt(tan(t/280)-(9+lr)%t*tan(t/(40+lr))%tan(t/(80+lr)))):pat2=='2'?sin(cbrt(tan(t/280)-(9+lr)%t*tan(t/(54+lr))%tan(t/(70+lr)))):pat2=='3'?((tan(sin(t/(280+lr)))*tan(t/(280+lr))*cbrt(t/20))%sin(t/2047))*3:pat2=='4'?sin(tan(t/(160+lr))+tan(t/(80+lr)))*(-t>>5&255)/90:pat2=='5'?sin(tan(t/(160+lr))+tan(t/(240+lr)))*(-t>>5&255)/180:0),
[atan(drums(0.01)+bass(-0.01)),atan(drums(-0.01)+bass(0.01))]