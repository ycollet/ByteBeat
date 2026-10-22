f=a=>sin(t%541/8e3*a),p=a=>{q=0;a.forEach(b=>q+=f(b)/a.length);return q},DTMF=[
[312,2497],[520,1995],[681,1659],
[558,1201],[335,865]],(p(DTMF[(t>>13&-t>>12)%DTMF.length])*128)%256*(1-t%541/541)