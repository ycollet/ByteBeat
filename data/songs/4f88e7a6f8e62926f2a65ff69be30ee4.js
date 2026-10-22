t||(a=0),
t||(S=max(40,random()*200),
	n=Array(l=256*round(S)).fill(0)),
x=((X=t/S/256+abs(sin(t*PI/8388608*S))*8)>>0)&7,
A=sin(sin(sin(a+=PI/24e3*320*2**([0,3,5,7,10,12,15,19][x]/12))+a*2))/8*min(.01,X%1)*(1-X%1)*100+sin(sin(b=t*PI/48e3*320/4)*2+b*2)/4*min(.01,X/8%1)*(1-X/8%1)*120+(random()-.5)*((X&7)==0)/8*sin(X*PI)+n[t%l]/1.2,n[t%l]=A,(A+sin(pow(1e-3,X/8%1)*PI*32)/2)/3