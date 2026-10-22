f=x=>sin(t*x*PI/256)*(-(t/x)>>5&255)/256,g=(x)=>x&&g(x-1)+f(x), pad = n => String(n % 100).padStart(2, '0'),

T=x=>{throw x},t&8191?g(8)/8:T('\n'+`  ${t>>13}\n`+Array.from({length:8},(_,i)=>i+1).map(X=>pad(X)+Array.from({length:32},(_,i)=>(((t>>13)+i)%(X)==0?'@':' ')).reduce((a,b)=>(a+b))).reduce((a,b)=>a+'\n'+b))