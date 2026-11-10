'use strict';
// A read-only device view independent of the selected document or delivery review.
window.JripPrinterDashboard=(()=>{
 const words={en:{dashboardConnecting:'Connecting updates',dashboardConnected:'Following saved reports',dashboardRetrying:'Updates paused · reconnecting',dashboardDenied:'Updates unavailable',printerDashboard:'Printers',noPrinters:'No printers configured',dashboardHelp:'Saved device reports. Refresh queries the selected printer; this does not print, cancel, approve or change an output review.'},ja:{dashboardConnecting:'通知に接続中',dashboardConnected:'保存報告を購読中',dashboardRetrying:'通知停止・再接続中',dashboardDenied:'通知を利用できません',printerDashboard:'プリンター',noPrinters:'登録済みプリンターなし',dashboardHelp:'保存済みの機器報告です。更新ボタンで選択機器へ照会します。印刷・取消・承認・出力条件の確認変更は行いません。'},'zh-CN':{dashboardConnecting:'正在连接通知',dashboardConnected:'正在订阅保存报告',dashboardRetrying:'通知暂停・正在重连',dashboardDenied:'无法获取通知',printerDashboard:'打印机',noPrinters:'未配置打印机',dashboardHelp:'已保存的设备报告。刷新会查询所选设备，不打印、取消、审批或修改输出确认。'}};
 for(const lang of Object.keys(words))Object.assign(texts[lang],words[lang]);
 const panel=$('printer-dashboard'),select=$('dashboard-printer');
 let owner=null,version=0,snapshot=null,error=false,loading=false,timer=null,stopStream=null,streamState='';
 const valid=(s,v)=>s===session&&s===owner&&v===version&&panel.open;
 function stopUpdates(){stopStream?.();stopStream=null;streamState='';}
 function updates(s,v,id){
  if(!valid(s,v)||!id)return;
  stopUpdates();
  stopStream=window.JripPrinterStream.subscribe({printerId:id,session:s,
   onState(value){if(valid(s,v)&&id===select.value){streamState=value;render();}},
   onSnapshot(value){if(!valid(s,v)||id!==select.value)return;const latest=value.latest;
    snapshot=latest?.snapshot&&Number.isFinite(latest.lastObservedAt)?{id,live:latest.snapshot,observedAt:latest.lastObservedAt,expiresAt:latest.expiresAt,stored:true}:null;
    error=Boolean(latest?.queryFailure);render();
   }
  });
 }
 function render(){
  const hint=$('dashboard-updates');hint.hidden=!select.value||!streamState;hint.textContent=streamState?t({connecting:'dashboardConnecting',connected:'dashboardConnected',retrying:'dashboardRetrying',denied:'dashboardDenied'}[streamState]):'';
  window.JripPrinterStatus.render({snapshot,error,loading,selected:select.value,prefix:'dashboard-'});
  if(select.value&&!snapshot&&!loading&&!error)$('dashboard-printer-query-state').textContent=t('S_UNREPORTED');
  $('dashboard-printer-refresh').disabled=loading||!session||!select.value;
  $('dashboard-list-refresh').disabled=loading||!session;
  const help=panel.querySelector('[data-title="printerStatusHelp"]');help.title=t('dashboardHelp');help.setAttribute('aria-label',help.title);
 }
 function reset(){stopUpdates();version++;clearTimeout(timer);timer=null;owner=null;snapshot=null;error=false;loading=false;select.replaceChildren();panel.hidden=true;panel.open=false;render();}
 function schedule(){clearTimeout(timer);if(owner&&panel.open)timer=setTimeout(()=>{render();schedule();},3000);}
 async function read(live=false){
  stopUpdates();const s=session,id=select.value,v=++version;snapshot=null;error=false;loading=Boolean(id);render();if(!s||!id)return;
  try{
   const result=await json(`/api/v1/printers/${encodeURIComponent(id)}/observations?limit=1`,{},s);
   if(!valid(s,v)||id!==select.value)return;
   const latest=result.latest;
   if(latest?.snapshot?.profile==='org.jrip.printer-state-snapshot-v1'&&Number.isFinite(latest.lastObservedAt))snapshot={id,live:latest.snapshot,observedAt:latest.lastObservedAt,expiresAt:latest.expiresAt,stored:true};
   error=Boolean(latest?.queryFailure);render();
  }catch(e){if(!valid(s,v))return;error=true;if(!live){loading=false;render();updates(s,v,id);report(e);return;}}
  if(live&&valid(s,v)&&id===select.value){
   try{
    const result=await json(`/api/v1/print/capabilities?printerId=${encodeURIComponent(id)}`,{},s);
    if(!valid(s,v)||id!==select.value)return;
    const device=result.stages.find(stage=>stage.capability.stage==='printer')?.capability.extensions['org.jrip.printer'];
    snapshot={id,live:device,observedAt:result.observedAt,expiresAt:result.expiresAt};error=false;
   }catch(e){if(!valid(s,v))return;error=true;report(e);}
  }
  if(valid(s,v)){loading=false;render();schedule();updates(s,v,id);}
 }
 async function load(){
  stopUpdates();const s=session,v=++version,previous=select.value;loading=true;render();
  try{
   const result=await json('/api/v1/printers',{},s);if(!valid(s,v))return;
   select.replaceChildren(new Option(t(result.printers.length?'choosePrinter':'noPrinters'),''));
   for(const printer of result.printers)select.append(new Option(printer.id,printer.id));
   if([...select.options].some(o=>o.value===previous))select.value=previous;
   snapshot=null;error=false;loading=false;await read();schedule();
  }catch(e){if(valid(s,v)){snapshot=null;error=true;loading=false;select.replaceChildren();render();report(e);}}
 }
 function translateDashboard(){
  if(owner!==session){reset();owner=session;panel.hidden=!session;if(session&&panel.open)load();}
  const empty=select.querySelector('option[value=""]');if(empty)empty.textContent=t(select.options.length>1?'choosePrinter':'noPrinters');render();
 }
 panel.addEventListener('toggle',()=>{if(panel.open&&session){owner=session;load();}else{stopUpdates();version++;loading=false;clearTimeout(timer);render();}});
 select.onchange=()=>read();$('dashboard-printer-refresh').onclick=()=>read(true);$('dashboard-list-refresh').onclick=load;
 return {reset,translate:translateDashboard};
})();
translate();
