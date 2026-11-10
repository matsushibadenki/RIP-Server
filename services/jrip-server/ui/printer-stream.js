'use strict';
// Bounded authenticated SSE reader. Cursor and credentials live only in memory.
window.JripPrinterStream=(()=>{
 function validId(id){const parts=id.split(':');return parts.length===2&&/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(parts[0])&&/^(0|[1-9][0-9]{0,18})$/.test(parts[1])&&BigInt(parts[1])<=9223372036854775807n;}
 function subscribe({printerId,session:owner,onSnapshot,onState}){
  let stopped=false,controller=null,retryTimer=null,retryDone=null,cursor='',backoff=1000;
  function stop(){if(stopped)return;stopped=true;controller?.abort();clearTimeout(retryTimer);retryDone?.();owner.abort.signal.removeEventListener('abort',stop);}
  owner.abort.signal.addEventListener('abort',stop,{once:true});
  if(owner.abort.signal.aborted){stop();return stop;}
  async function run(){
   while(!stopped){
    controller=new AbortController();let reader;
    onState('connecting');
    try{
     const response=await request(`/api/v1/printers/${encodeURIComponent(printerId)}/observations/stream`,{signal:controller.signal,headers:{Accept:'text/event-stream',...(cursor?{'Last-Event-ID':cursor}:{})}},owner);
     if(!response.headers.get('content-type')?.startsWith('text/event-stream')||!response.body)throw Error('invalid stream');
     reader=response.body.getReader();const decoder=new TextDecoder('utf-8',{fatal:true});let buffer='';onState('connected');
     while(!stopped){
      const chunk=await reader.read();if(chunk.done)throw Error('stream ended');
      buffer=(buffer+decoder.decode(chunk.value,{stream:true})).replaceAll('\r\n','\n');
      let end;
      while((end=buffer.indexOf('\n\n'))!==-1){
       const frame=buffer.slice(0,end);buffer=buffer.slice(end+2);if(frame.length>524288)throw Error('oversized frame');
       let kind='message',id=null;const data=[];
       for(const line of frame.split('\n')){if(line.startsWith(':'))continue;const colon=line.indexOf(':');const field=colon===-1?line:line.slice(0,colon);let value=colon===-1?'':line.slice(colon+1);if(value.startsWith(' '))value=value.slice(1);if(field==='event')kind=value;else if(field==='id')id=value;else if(field==='data')data.push(value);}
       if(!data.length)continue;
       if(kind==='stream-error'||kind==='stream-end'){
        const value=JSON.parse(data.join('\n'));
        if(value.reasonCode==='AUTHORIZATION_EXPIRED'){onState('denied');stop();if(owner===session){disconnect();notify('expired');}return;}
        throw Error('reconnect required');
       }
       if(kind!=='printer-observation')continue;
       const value=JSON.parse(data.join('\n')),latest=value.latest;
       if(value.profile!=='org.jrip.printer-observation-stream-v1'||value.printerId!==printerId||id===null)throw Error('invalid observation');
       if(latest===null){if(id!=='')throw Error('invalid empty cursor');}
       else if(!latest||!validId(id)||id.split(':')[0]!==value.streamId||latest.streamId!==value.streamId||latest.printerId!==printerId||latest.profile!=='org.jrip.printer-observations-v1'||(latest.snapshot!==null&&latest.snapshot?.profile!=='org.jrip.printer-state-snapshot-v1'))throw Error('invalid snapshot');
       if(stopped)return;onSnapshot(value);cursor=id;backoff=1000;
      }
      if(buffer.length>524288)throw Error('oversized frame');
     }
    }catch(error){
     if(stopped||owner.abort.signal.aborted)return;
     if(error.status===401||error.status===403){onState('denied');stop();return;}
     onState('retrying');
    }finally{controller?.abort();if(reader){try{await reader.cancel();}catch{}reader.releaseLock();}}
    if(stopped)return;
    await new Promise(resolve=>{retryDone=resolve;retryTimer=setTimeout(resolve,backoff);});retryDone=null;backoff=Math.min(backoff*2,30000);
   }
  }
  void run();return stop;
 }
 return {subscribe};
})();
