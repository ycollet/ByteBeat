(
  t?0:(_T=Date.now(),_S=t),
  rate = (t%1000==0 && t>0) ? Math.round((t-_S)/((Date.now()-_T)/1000)) : (window.sampleRate||8000),

 // WHERE YOU SET YOUR ALARM PREFERENCES
  AH=9, AM=0, 
  //AH = AlarmHour
  //AM = AlarmMinute
  // ====================================
  n=new Date(), h=n.getHours(), m=n.getMinutes(), s=n.getSeconds(), ms=n.getMilliseconds(),
  isA=(h==AH && m==AM),
  (t % rate > 500) ? 0 : (()=>{
    res="", Y=6;
    while(Y>=-6){
      X=-12, row="";
      while(X<=12){
        dx=X/2, dy=Y, d=Math.sqrt(dx*dx+dy*dy), c=" ";
        aS=s*Math.PI/30, aM=(m+s/60)*Math.PI/30, aH=(h%12+m/60)*Math.PI/6;
        if(d>5.2 && d<7){
          if(dy>4.5)c="1"; else if(dx>5.5)c="3";
          else if(dy<-4.5)c="6"; else if(dx<-5.5)c="9";
        }
        if(d<6 && Math.abs(dx*Math.cos(aS)-dy*Math.sin(aS))<0.4 && (dx*Math.sin(aS)+dy*Math.cos(aS))>0){
          ang=(aS*180/Math.PI)%180;
          c=(ang<22||ang>157)?"|":(ang<67)?"/":(ang<112)?"-":"x";
        }
        if(d<4.5 && Math.abs(dx*Math.cos(aM)-dy*Math.sin(aM))<0.5 && (dx*Math.sin(aM)+dy*Math.cos(aM))>0) c="M";
        if(d<3 && Math.abs(dx*Math.cos(aH)-dy*Math.sin(aH))<0.7 && (dx*Math.sin(aH)+dy*Math.cos(aH))>0) c="H";
        if(dx==0 && dy==0) c="+";
        row+=c, X++;
      }
      res+=row+"\n", Y--;
    }
    throw "\n[ bluethefox ]\n"+res+"TIME: "+h+":"+m+":"+s+"\nALARM: "+AH+":"+AM+"\nSTATUS: "+(isA?"WAKEUP":"ARMED")+"\nRATE: "+rate+"Hz"
  })()
),
isA ? 
  (sin(t*(1600/rate)*(0.2+sin(t*(80/rate))*0.15))*127+128) : 
  (s==0 && ms<1000 ? 
    (sin(t*(400/rate))*(127-(t%rate)/(rate/127))+128) : 
    (ms < 50 ? (sin(t*(4800/rate))*(127-(t%(rate/20))/(rate/2540))+128) : 128)
  )

//the most weird alarm clock ever
//by bluethefox weirdo who does weird things
