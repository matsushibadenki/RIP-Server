import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
const source=await readFile(new URL('../../services/jrip-server/ui/printer-stream.js',import.meta.url),'utf8');
const uuid='12345678-1234-1234-1234-123456789abc';
function payload(serial=1){return {profile:'org.jrip.printer-observation-stream-v1',printerId:'p1',streamId:uuid,latest:{profile:'org.jrip.printer-observations-v1',printerId:'p1',streamId:uuid,querySerial:serial,snapshot:{profile:'org.jrip.printer-state-snapshot-v1',note:'紙詰まり・卡纸'}}};}
function frame(serial=1){return `: keep-alive\r\n\r\nevent: printer-observation\r\nid: ${uuid}:${serial}\r\ndata: ${JSON.stringify(payload(serial))}\r\n\r\n`;}
function environment(request){const owner={abort:new AbortController()};const delays=[];const context={window:{},request,TextDecoder,AbortController,session:owner,disconnect(){},notify(){},setTimeout(fn,ms){delays.push(ms);return setTimeout(fn,Math.min(ms,5));},clearTimeout};vm.runInNewContext(source,context);return {api:context.window.JripPrinterStream,owner,delays,context};}
function response(text,close=true,onCancel=()=>{},fragment=3){const bytes=new TextEncoder().encode(text);return new Response(new ReadableStream({start(c){for(let i=0;i<bytes.length;i+=fragment)c.enqueue(bytes.slice(i,i+fragment));if(close)c.close();},cancel:onCancel}),{headers:{'content-type':'text/event-stream'}});}
function deadline(promise){return Promise.race([promise,new Promise((_,reject)=>{const timer=setTimeout(()=>reject(Error('stream test timeout')),3000);timer.unref();})]);}
test('fragmented UTF-8 and CRLF frames resume with the last accepted cursor',async()=>{
 const calls=[],values=[],states=[];let done;const completed=new Promise(r=>done=r);
 const {api,owner,delays}=environment(async(url,options)=>{calls.push({url,options});return response(frame(calls.length));});
 let stop;stop=api.subscribe({printerId:'p1',session:owner,onState:s=>states.push(s),onSnapshot:value=>{values.push(value);if(values.length===2){stop();done();}}});
 await deadline(completed);assert.equal(values[0].latest.snapshot.note,'紙詰まり・卡纸');assert.equal(values[1].latest.querySerial,2);
 assert.equal(calls.length,2);assert.equal(calls[0].options.headers['Last-Event-ID'],undefined);assert.equal(calls[1].options.headers['Last-Event-ID'],`${uuid}:1`);assert.equal(calls[0].url.includes('token'),false);assert(states.includes('retrying'));assert.equal(delays[0],1000);
});
test('oversized and wrong-device frames do not advance cursors, and forbidden stops retries',async()=>{
 let calls=0,cancelled=0,done;const completed=new Promise(r=>done=r);
 const {api,owner,delays}=environment(async(_url,options)=>{
  calls++;assert.equal(options.headers['Last-Event-ID'],undefined);
  if(calls===1)return response('x'.repeat(524289),false,()=>cancelled++,524289);
  if(calls===2)return response(frame(2).replaceAll('"p1"','"p2"'),false,()=>cancelled++);
  const error=Error('forbidden');error.status=403;throw error;
 });
 api.subscribe({printerId:'p1',session:owner,onSnapshot:()=>assert.fail('malformed data was rendered'),onState:s=>{if(s==='denied')done();}});
 await deadline(completed);await new Promise(r=>setTimeout(r,20));assert.equal(calls,3);assert.equal(cancelled,2);assert.deepEqual(delays,[1000,2000]);
});
test('session abort cancels an open body and prevents reconnects',async()=>{
 let calls=0,done;const cancelled=new Promise(r=>done=r);let owner;
 const env=environment(async()=>{calls++;return response(': keep-alive\n\n',false,done);});owner=env.owner;
 env.api.subscribe({printerId:'p1',session:owner,onSnapshot:()=>assert.fail('unexpected snapshot'),onState:s=>{if(s==='connected')owner.abort.abort();}});
 await deadline(cancelled);await new Promise(r=>setTimeout(r,20));assert.equal(calls,1);assert.equal(env.delays.length,0);
});

test('empty reports clear resume cursors and authorization-expiry clears the session',async()=>{
 let calls=0,done;const completed=new Promise(r=>done=r);
 const empty='event: printer-observation\nid:\ndata: '+JSON.stringify({profile:'org.jrip.printer-observation-stream-v1',printerId:'p1',streamId:null,latest:null})+'\n\n';
 const env=environment(async(_url,options)=>{calls++;if(calls===2)assert.equal(options.headers['Last-Event-ID'],undefined);return response(calls===1?frame()+empty:frame(2));});
 env.api.subscribe({printerId:'p1',session:env.owner,onState(){},onSnapshot:value=>{if(value.latest?.querySerial===2){env.owner.abort.abort();done();}}});
 await deadline(completed);assert.equal(calls,2);
 let expired;const observed=new Promise(r=>expired=r);const auth=environment(async()=>response('event: stream-error\ndata: {"reasonCode":"AUTHORIZATION_EXPIRED"}\n\n'));
 let disconnected=0;auth.context.disconnect=()=>{disconnected++;auth.owner.abort.abort();};auth.context.notify=reason=>{assert.equal(reason,'expired');expired();};
 auth.api.subscribe({printerId:'p1',session:auth.owner,onState(){},onSnapshot:()=>assert.fail('expired snapshot rendered')});
 await deadline(observed);assert.equal(disconnected,1);assert.equal(auth.owner.abort.signal.aborted,true);assert.equal(auth.delays.length,0);
});
