'use strict';
// M3 Expressive tokens: fast spatial (800, .6), default spatial (380, .8),
// default effects (1600, 1). Mass=1 and the integrator are web implementation choices.
const motionPreference=matchMedia('(prefers-reduced-motion: reduce)');
const springTracks=new Map();
let springFrame=0;
function applySpringProperty(element,key,value){
  if(key==='radius')element.style.borderRadius=Math.max(0,value)+'px';
  if(key==='grow')element.style.flexGrow=Math.max(.1,value);
  if(key==='y')element.style.transform=`translateY(${value}px)`;
  if(key==='opacity')element.style.opacity=Math.max(0,Math.min(1,value));
}
function spring(element,values,stiffness=800,damping=.6){
  let tracks=springTracks.get(element);
  if(!tracks){tracks=new Map();springTracks.set(element,tracks)}
  for(const [key,target] of Object.entries(values)){
    const previous=tracks.get(key);
    let initial=target;
    if(key==='radius')initial=Math.min(parseFloat(getComputedStyle(element).borderTopLeftRadius)||0,element.offsetHeight/2);
    if(key==='grow')initial=parseFloat(getComputedStyle(element).flexGrow)||1;
    if(key==='y')initial=Number(element.dataset.motionY||0);
    if(key==='opacity')initial=parseFloat(getComputedStyle(element).opacity);
    if(motionPreference.matches){applySpringProperty(element,key,target);tracks.delete(key);continue}
    tracks.set(key,{position:previous?.position??initial,velocity:previous?.velocity??0,target,stiffness,damping});
  }
  if(!tracks.size)springTracks.delete(element);
  if(!springFrame&&springTracks.size)springFrame=requestAnimationFrame(tickSprings);
}
let previousSpringTime=0;
function tickSprings(now){
  const elapsed=previousSpringTime?Math.min((now-previousSpringTime)/1000,.032):1/60;
  previousSpringTime=now;
  const steps=Math.ceil(elapsed/(1/120)),dt=elapsed/steps;
  for(const [element,tracks] of springTracks){
    if(!element.isConnected){springTracks.delete(element);continue}
    for(const [key,track] of tracks){
      for(let step=0;step<steps;step++){
        const force=-track.stiffness*(track.position-track.target)-2*track.damping*Math.sqrt(track.stiffness)*track.velocity;
        track.velocity+=force*dt;
        track.position+=track.velocity*dt;
      }
      if(Math.abs(track.position-track.target)<.015&&Math.abs(track.velocity)<.03){track.position=track.target;tracks.delete(key)}
      applySpringProperty(element,key,track.position);
    }
    if(!tracks.size)springTracks.delete(element);
  }
  springFrame=springTracks.size?requestAnimationFrame(tickSprings):0;
  if(!springFrame)previousSpringTime=0;
}
motionPreference.addEventListener('change',()=>{
  if(!motionPreference.matches)return;
  cancelAnimationFrame(springFrame);springFrame=0;previousSpringTime=0;
  for(const [element,tracks] of springTracks)for(const [key,track] of tracks)applySpringProperty(element,key,track.target);
  springTracks.clear();
});
function restingRadius(button){
  const half=button.offsetHeight/2;
  if(button.id==='scanToggle'&&button.getAttribute('aria-pressed')==='true')return half>30?28:16;
  if(button.hasAttribute('data-demo'))return button.getAttribute('aria-pressed')==='true'?16:half;
  if(button.classList.contains('chip'))return button.classList.contains('active')?half:8;
  return half;
}
function pressable(target){return target.closest('button:not(.device):not(.navitem):not(.char):not(.tab):not(.ghost)')}
function pressButton(button){
  if(!button||button.disabled)return;
  const height=button.offsetHeight;
  spring(button,{radius:height>64?16:12});
  const group=button.closest('.standard-group');
  if(group){
    const siblings=[...group.querySelectorAll('button')],index=siblings.indexOf(button);
    siblings.forEach((sibling,i)=>spring(sibling,{grow:i===index?1.18:Math.abs(i-index)===1?.82:1}));
  }
}
function releaseButton(button){
  if(!button)return;
  spring(button,{radius:restingRadius(button)});
  button.closest('.standard-group')?.querySelectorAll('button').forEach(sibling=>spring(sibling,{grow:1}));
}
let heldButton=null;
document.addEventListener('pointerdown',event=>{heldButton=pressable(event.target);pressButton(heldButton)});
for(const event of ['pointerup','pointercancel'])document.addEventListener(event,()=>{releaseButton(heldButton);heldButton=null});
window.addEventListener('blur',()=>{releaseButton(heldButton);heldButton=null});
document.addEventListener('keydown',event=>{if((event.key===' '||event.key==='Enter')&&!event.repeat){heldButton=pressable(event.target);pressButton(heldButton)}});
document.addEventListener('keyup',event=>{if(event.key===' '||event.key==='Enter'){releaseButton(heldButton);heldButton=null}});
function enterSurface(element){
  if(!element||motionPreference.matches)return;
  const tracks=springTracks.get(element);tracks?.delete('y');tracks?.delete('opacity');
  element.dataset.motionY='18';element.style.transform='translateY(18px)';element.style.opacity='0';
  spring(element,{y:0},380,.8);spring(element,{opacity:1},1600,1);
}
document.addEventListener('click',event=>{
  const button=event.target.closest('button');
  if(!button)return;
  if(button.hasAttribute('data-page')||button.hasAttribute('data-device')||button.id==='backDevices')enterSurface(document.querySelector('.page.active'));
  if(button.classList.contains('chip'))document.querySelectorAll('.chip').forEach(item=>spring(item,{radius:restingRadius(item)}));
  if(button.id==='scanToggle')spring(button,{radius:restingRadius(button)});
});
document.getElementById('motionOpen').onclick=()=>{document.getElementById('motionDialog').showModal();enterSurface(document.getElementById('motionDialog'))};
document.getElementById('motionClose').onclick=()=>document.getElementById('motionDialog').close();
document.querySelectorAll('[data-demo]').forEach(button=>button.onclick=()=>{
  const selected=button.getAttribute('aria-pressed')!=='true';
  document.querySelectorAll('[data-demo]').forEach(item=>{item.setAttribute('aria-pressed',String(item===button&&selected));spring(item,{radius:restingRadius(item)})});
  document.getElementById('motionResult').textContent=selected?'已选择“'+button.dataset.demo+'”演示动作':'未选择演示动作';
});
document.getElementById('themeToggle').onclick=()=>{
  const dark=document.documentElement.dataset.theme!=='dark';
  document.documentElement.dataset.theme=dark?'dark':'light';
  document.getElementById('themeToggle').textContent=dark?'浅色模式':'深色模式';
  document.getElementById('themeToggle').setAttribute('aria-pressed',String(dark));
};
document.getElementById('export').setAttribute('aria-label','导出调试会话');
