SR=16e3;TAU=PI*2;
scale=[0,2,4,5,7,9,11];
mod=(n,m=1)=>(n%m+m)%m;
rot=(sc,m,n)=>sc.map(v=>mod(v+n,m)).sort((a,b)=>a-b);
nHz=n=>432*2**(n/12);
rand=(a=1,b)=>b?a+random()*(b-a):random()*a;
pickRand=a=>a[floor(random()*a.length)];
mix=(a,b,t)=>a+(b-a)*t;
makeWave=()=>{
 let a=[];for(let len=floor(rand(3,10)),i=0;i<len;i++)a.push(rand());
 let mn=min(...a),mx=max(...a);
 a=a.map(v=>((v-mn)/(mx-mn))*2-1);
 return t=>mix(a[mod(floor(t*a.length),a.length)],a[mod(floor(t*a.length)+1,a.length)],mod(t*a.length));
};
class Pattern{
 constructor(speed=1){
  Object.assign(this,{p:[],d:12,timer:{atIdx:0,tick:0,bar:0},voices:[],speed,keyShift:0});
  this.generate();
 }
 generate(){
  this.p=[];
  this.keyShift=mod(this.keyShift+pickRand([5,7])*(rand()<.5?1:-1),12);
  this.scale=rot(scale,12,this.keyShift);
  for(let i=0;i<32;i++)
   this.p.push({s:floor(rand(this.d)),l:rand(.1,1),p:pickRand(this.scale)+12*(rand()<.5)-11});
  this.p.sort((a,b)=>a.s-b.s);
 }
 process(){
  for(let i=this.timer.atIdx;i<this.p.length;i++){
   let note=this.p[i];
   if(this.timer.tick>=note.s){
    this.voices.push({atk:rand(.08,.1),rel:rand(.1,7),l:note.l,p:note.p+rand(-1,1)*.1,t:0,phase:rand(),vel:rand(.1,1),wave:makeWave(),LFO:{a:rand(.5),sp:rand(15),t:0}});
    this.timer.atIdx++;
   }else{
    break;
   }
  }
  this.timer.tick+=this.speed/SR;
  if(this.timer.tick>=this.d){
   this.timer.tick%=this.d;this.timer.atIdx=0;this.timer.bar++;
   if(this.timer.bar>=4){this.timer.bar=0;this.generate();}
  }
  let o=0;
  for(let i=this.voices.length-1;i>=0;i--){
   let v=this.voices[i];
   v.phase=(v.phase+(nHz(v.p+sin(v.LFO.t*TAU)*v.LFO.a)/SR))%1;
   v.LFO.t=(v.LFO.t+(v.LFO.sp/SR))%1;
   o+=v.wave(v.phase)*v.vel*min(v.t/v.atk,1,1-((v.t-v.l)/v.rel));
   v.t+=this.speed/SR;
   if(v.t>=v.l+v.rel)this.voices.splice(i,1);
  }
  return o;
 }
}
myP=new Pattern(10.5);
return t=>{
 return myP.process()*.3;
};