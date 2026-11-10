import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
const source=fs.readFileSync(new URL('../../services/jrip-server/ui/app.js',import.meta.url),'utf8');
const showPage=source.slice(source.indexOf('async function showPage(){'),source.indexOf('async function control('));
async function run(failures,changeSelection=false){
 let calls=0,reports=[],delays=[],elements={page:{value:'page-000001.tiff'},'preview-image':{},'preview-message':{}};
 const context={selected:{id:'job'},session:{},previewVersion:0,previewURL:null,
 $:id=>elements[id],resetPreview(){context.previewVersion++},URL:{createObjectURL:()=> 'blob:preview'},
 request:async()=>{calls++;if(failures.length)throw failures.shift();return {blob:async()=>new Blob(['BMP'])}},
 setTimeout(callback,delay){delays.push(delay);if(changeSelection)context.previewVersion++;callback()},
 report:error=>reports.push(error),encodeURIComponent};
 vm.createContext(context);await vm.runInContext(showPage+';showPage()',context);
 return {calls,reports,delays,elements};
}
const busy=()=>({status:503,code:'artifact_budget_exhausted'});
let result=await run([busy(),busy()]);assert.equal(result.calls,3);assert.deepEqual(result.delays,[250,500]);assert.equal(result.elements['preview-image'].src,'blob:preview');assert.equal(result.reports.length,0);
result=await run([busy(),busy(),busy(),busy()]);assert.equal(result.calls,4);assert.equal(result.reports.length,1);
result=await run([{status:403}]);assert.equal(result.calls,1);assert.equal(result.reports.length,1);
result=await run([{status:422}]);assert.equal(result.calls,1);assert.equal(result.reports.length,0);
result=await run([busy()],true);assert.equal(result.calls,1);assert.equal(result.elements['preview-image'].src,undefined);
console.log('PASS: transient recovery, bounded exhaustion, non-retry errors, stale selection');
