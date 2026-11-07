import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import {mkdir} from 'node:fs/promises';
const {JRIP_UI_URL:url,JRIP_UI_TOKEN:token,JRIP_UI_COLOR:color}=process.env;
assert(url&&token&&color,'Use the isolated Rust delivery browser test');
const browser=await chromium.launch({headless:true,executablePath:process.env.JRIP_CHROMIUM_PATH||undefined});
const context=await browser.newContext({viewport:{width:1440,height:1000},locale:'en-US'});
const page=await context.newPage();const errors=[];const writes=[];
page.on('pageerror',error=>errors.push(error.message));
page.on('console',message=>{if(['warning','error'].includes(message.type())&&!/Failed to load resource: (net::ERR_FAILED|the server responded with a status of (502|503))/.test(message.text()))errors.push(message.text());});
page.on('request',request=>{if(request.method()==='POST'&&new URL(request.url()).pathname.endsWith('/deliveries'))writes.push(JSON.parse(request.postData()));});
const wait=async(fn,label)=>{for(let n=0;n<200;n++){if(await fn())return;await page.waitForTimeout(100);}throw new Error(label);};
const api=async path=>{const response=await context.request.get(new URL(path,url).href,{headers:{Authorization:`Bearer ${token}`}});assert.equal(response.status(),200);return response.json();};
async function connect(value){await page.locator('#connection').click();await page.locator('#token').fill(value);await page.locator('#login-form button[type=submit]').click();await wait(()=>page.locator('#login').isVisible().then(v=>!v),'connected');}
async function openOutput(){await page.getByRole('button',{name:'test.pdf',exact:true}).click();await wait(()=>page.locator('#delivery-panel').isVisible(),'physical controls visible');await page.locator('#delivery-panel > summary').click();await wait(()=>page.locator('#delivery-printer').isEnabled(),'destinations loaded');}
async function configure(copies='3',ranges='2-3',sides='two-sided-long-edge'){
 await page.locator('#delivery-printer').selectOption('test');await wait(()=>page.locator('#delivery-media').isEnabled(),'live capabilities loaded');
 assert.equal(await page.locator('#delivery-media').inputValue(),'custom_square_25.4x25.4mm');
 assert.deepEqual(await page.locator('#delivery-sides option').evaluateAll(options=>options.map(o=>o.value)),['one-sided','two-sided-long-edge','two-sided-short-edge']);
 assert.equal(await page.locator('#delivery-copies').getAttribute('max'),'5');
 await page.locator('#delivery-color').selectOption(color);await page.locator('#delivery-copies').fill(copies);await page.locator('#delivery-pages').fill(ranges);await page.locator('#delivery-sides').selectOption(sides);
 await page.locator('#delivery-check').click();await wait(()=>page.locator('#delivery-review').isVisible(),'verified output review');
}
async function approve(){await page.locator('#delivery-reason').fill('Review of exact selected pages and media');await page.locator('#delivery-approve').click();await wait(()=>page.locator('#delivery-approval').inputValue().then(v=>v.length===36),'exact approval created');await wait(()=>page.locator('#delivery-send').isEnabled(),'approval finished');}
try{
 const screenshots=process.env.JRIP_UI_SCREENSHOTS;if(screenshots)await mkdir(screenshots,{recursive:true});
 await page.goto(url);assert.equal(await page.title(),'J-RIP');assert.equal(new URL(page.url()).pathname,'/console');

 await connect(token);if(screenshots)await page.screenshot({path:`${screenshots}/printer-first-viewport.png`,fullPage:false});await openOutput();
 const printerRoute=new URL('/api/v1/print/capabilities?printerId=test',url).href;
 // The Rust fixture has reopened the repository after saving this report.
 const savedPath='/api/v1/printers/test/observations?limit=1';
 const saved=await api(savedPath);assert.equal(saved.latest.snapshot.state,5);
 const countBefore=(await (await context.request.get(new URL('/__test/queries',url).href)).json()).count;
 let releaseLive;const liveGate=new Promise(resolve=>{releaseLive=resolve;});
 await page.route(printerRoute,async handler=>{await liveGate;await handler.fulfill({status:503,contentType:'application/json',body:JSON.stringify({error:{code:'printer_unavailable'}})});});
 await page.locator('#delivery-printer').selectOption('test');
 await wait(()=>page.locator('#printer-live').getAttribute('data-source').then(v=>v==='stored'),'saved report appears before live query finishes');
 assert.equal(await page.locator('#printer-live').getAttribute('aria-busy'),'true');
 releaseLive();
 await wait(()=>page.locator('#printer-refresh').isEnabled(),'saved fallback finishes');
 assert.equal(await page.locator('#printer-live').getAttribute('data-source'),'stored');
 assert((await page.locator('#printer-query-state').innerText()).includes('Saved report'));
 assert((await page.locator('#printer-conditions').innerText()).includes('Paper jam'));
 assert((await page.locator('#printer-cups-supplies').innerText()).includes('0 %'));
 assert.equal(await page.locator('#delivery-check').isDisabled(),true,'saved state cannot authorize a capability review');
 // A full page reload loses all JS memory; the same report must still appear.
 await page.reload();await connect(token);await openOutput();
 await page.locator('#delivery-printer').selectOption('test');await wait(()=>page.locator('#printer-refresh').isEnabled(),'fallback after browser reload');
 for(const [lang,label] of [['en','Saved report'],['ja','保存済みの報告'],['zh-CN','已保存报告']]){
  await page.locator('#language').selectOption(lang);
  assert((await page.locator('#printer-query-state').innerText()).includes(label));
  for(const width of [320,375,430,1440]){await page.setViewportSize({width,height:1000});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`stored fallback fits ${lang}/${width}`);}
 }
 await page.locator('#language').selectOption('en');await page.setViewportSize({width:1440,height:1000});
 assert.equal((await api(savedPath)).latest.lastObservedAt,saved.latest.lastObservedAt,'reading saved state does not renew observation time');
 assert.equal((await (await context.request.get(new URL('/__test/queries',url).href)).json()).count,countBefore,'cache reads do not query the device');
 if(screenshots)await page.locator('#printer-live').screenshot({path:`${screenshots}/printer-stored-after-restart.png`});
 const savedRoute=new URL(savedPath,url).href;
 await page.route(savedRoute,async handler=>{const response=await handler.fetch();const body=await response.json();body.latest.expiresAt=0;await handler.fulfill({response,json:body});});
 await page.locator('#printer-refresh').click();await wait(()=>page.locator('#printer-refresh').isEnabled(),'expired saved refresh');
 assert((await page.locator('#printer-query-state').innerText()).includes('Expired snapshot'));
 assert((await page.locator('#printer-query-state').innerText()).includes('Query unavailable'));
 assert((await page.locator('#printer-conditions').innerText()).includes('Paper jam'));
 await page.unroute(savedRoute);
 await page.route(savedRoute,handler=>handler.fulfill({status:200,contentType:'application/json',body:JSON.stringify({latest:null})}));
 await page.locator('#printer-refresh').click();await wait(()=>page.locator('#printer-refresh').isEnabled(),'empty saved refresh');
 assert.equal(await page.locator('#printer-snapshot-content').isHidden(),true,'a registration without saved evidence does not reuse another report');
 assert.equal(await page.locator('#delivery-check').isDisabled(),true);
 await page.unroute(savedRoute);
 await page.route(savedRoute,handler=>handler.fulfill({status:503,contentType:'application/json',body:'{}'}));
 await page.unroute(printerRoute);
 await page.locator('#printer-refresh').click();await wait(()=>page.locator('#printer-refresh').isEnabled(),'cache outage still permits live query');
 assert.equal(await page.locator('#printer-live').getAttribute('data-source'),'live');
 assert.equal(await page.locator('#delivery-media').isEnabled(),true);
 assert(!(await page.locator('#printer-query-state').innerText()).includes('Saved report'));
 await page.unroute(savedRoute);
 await page.locator('#delivery-printer').selectOption('');assert.equal(await page.locator('#printer-live').isHidden(),true);
 assert.equal(await page.locator('#printer-cups-supplies').textContent(),'','clearing destination removes saved data');
 const setPrinter=async data=>{const response=await fetch(new URL('/__test/printer',url),{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(data)});assert.equal(response.status,204);};
 const refreshPrinter=async()=>{await page.locator('#printer-refresh').click();await wait(()=>page.locator('#printer-refresh').isEnabled(),'printer refresh completed');};
 await setPrinter({state:5,reasons:['media-jam-error','toner-low-warning','vendor-needs-attention-report'],queuedJobs:0,markerMode:1,pwgMode:7});
 await page.locator('#delivery-printer').selectOption('test');await wait(()=>page.locator('#printer-live').isVisible()&&page.locator('#printer-refresh').isEnabled(),'printer telemetry loaded');
 assert.equal(writes.length,0,'telemetry reads never print');
 assert((await page.locator('#printer-conditions').innerText()).includes('Paper jam'));
 assert((await page.locator('#printer-facts').innerText()).includes('Device jobs: 0'));
 assert((await page.locator('#printer-pwg-supplies').innerText()).includes('25 %'));
 assert((await page.locator('#printer-pwg-supplies').innerText()).includes('Remaining space'));
 assert((await page.locator('#printer-cups-supplies').innerText()).includes('0 %'));
 assert.equal(await page.locator('#printer-cups-supplies meter').count(),2);
 for(const [lang,label] of [['en','Paper jam'],['ja','紙詰まり'],['zh-CN','卡纸']]){
  await page.locator('#language').selectOption(lang);assert((await page.locator('#printer-conditions').innerText()).includes(label));assert((await page.locator('#printer-pwg-supplies').innerText()).includes({en:'Waste toner',ja:'廃トナー','zh-CN':'废碳粉'}[lang]));
  assert((await page.locator('#printer-conditions').innerText()).includes('vendor-needs-attention-report'));
  assert((await page.locator('#printer-conditions button').first().getAttribute('title')).length>20,'recovery tooltip');
  for(const width of [320,375,430,1440]){await page.setViewportSize({width,height:1000});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`printer telemetry fits ${lang}/${width}`);}
  if(screenshots&&lang==='ja'){await page.locator('#printer-live').screenshot({path:`${screenshots}/printer-status-desktop-ja.png`});await page.setViewportSize({width:375,height:1000});await page.locator('#printer-live').screenshot({path:`${screenshots}/printer-status-mobile-ja.png`});}
 }
 await page.locator('#language').selectOption('en');await page.setViewportSize({width:1440,height:1000});
 await setPrinter({state:3,reasons:['none'],markerMode:2,pwgMode:2});await refreshPrinter();
 assert((await page.locator('#printer-facts').innerText()).includes('Device jobs: Unknown'));
 assert((await page.locator('#printer-cups-supplies').innerText()).includes('Unknown'));
 assert.equal(await page.locator('#printer-cups-supplies meter').count(),0,'unknown amounts are not zero meters');
 assert((await page.locator('#printer-pwg-supplies').innerText()).includes('2,000 items'));
 assert.equal(await page.locator('#printer-pwg-supplies meter').count(),0,'items are not percentages');
 await setPrinter({state:3,reasons:[],markerMode:3,pwgMode:4});await refreshPrinter();
 assert((await page.locator('#printer-cups-supplies').innerText()).includes('Invalid supply report'));
 assert((await page.locator('#printer-pwg-supplies').innerText()).includes('Invalid supply report'));
 assert((await page.locator('#printer-conditions').innerText()).includes('Notices not reported'));
 await setPrinter({state:3,reasons:[],markerMode:4,pwgMode:6});await refreshPrinter();assert((await page.locator('#printer-cups-supplies').innerText()).includes('Unknown'));assert((await page.locator('#printer-pwg-supplies').innerText()).includes('Unknown'));
 // Device-controlled descriptions are literal text; no element or handler is created.
 await page.route(printerRoute,async handler=>{const response=await handler.fetch();const body=await response.json();const live=body.stages.find(s=>s.capability.stage==='printer').capability.extensions['org.jrip.printer'];live.printerSupplies={status:'REPORTED',supplies:[{position:1,description:{text:'<img src=x onerror=alert(1)> '+ 'Long name '.repeat(20)},kind:'ink',levelMeaning:'CURRENT_SUPPLY',unit:'percent',level:0,levelPercent:0}]};body.expiresAt=body.observedAt-1;await handler.fulfill({response,json:body});});
 await refreshPrinter();assert.equal(await page.locator('#printer-live img').count(),0);assert((await page.locator('#printer-pwg-supplies').innerText()).includes('<img src=x'));
 await wait(()=>page.locator('#printer-query-state').innerText().then(v=>v.includes('Expired snapshot')),'expired telemetry is marked');
 await page.setViewportSize({width:320,height:1000});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'long names fit narrow layout');
 await page.unroute(printerRoute);await page.setViewportSize({width:1440,height:1000});
 await setPrinter({state:3,reasons:['none'],queuedJobs:7,markerMode:1,pwgMode:1});await refreshPrinter();
 await page.locator('#delivery-color').selectOption(color);await page.locator('#delivery-copies').fill('4');
 const prior=await page.locator('#printer-pwg-supplies').innerText();
 await page.route(printerRoute,handler=>handler.fulfill({status:503,contentType:'application/json',body:JSON.stringify({error:{code:'printer_unavailable'}})}));
 await refreshPrinter();assert((await page.locator('#printer-query-state').innerText()).includes('Query unavailable'));
 assert.equal(await page.locator('#printer-pwg-supplies').innerText(),prior,'failed read preserves clearly marked last report');
 await page.locator('#language').selectOption('ja');assert.equal(await page.locator('#delivery-color').inputValue(),color,'language change after failed refresh retains color');assert.equal(await page.locator('#delivery-copies').inputValue(),'4');assert.equal(await page.locator('#delivery-check').isDisabled(),true,'failed query blocks stale review');await page.locator('#language').selectOption('en');
 await page.unroute(printerRoute);await refreshPrinter();assert(!(await page.locator('#printer-query-state').innerText()).includes('Query unavailable'));
 await page.locator('#delivery-printer').selectOption('');assert.equal(await page.locator('#printer-live').isHidden(),true,'cleared destination hides prior device');assert.equal(await page.locator('#printer-pwg-supplies').textContent(),'','prior device data cleared');
 await configure('2');

 const job=(await api('/api/v1/jobs')).find(job=>job.manifest.name==='test.pdf');assert(job);
 assert.equal(writes.length,0);assert.deepEqual((await api(`/api/v1/jobs/${job.id}/deliveries`)).deliveries,[]);
 await page.locator('#delivery-send').click();await wait(()=>page.locator('#notice').innerText().then(v=>v.includes('approval')),'missing approval blocked');assert.equal(writes.length,0);
 await approve();await refreshPrinter();assert.equal(await page.locator('#delivery-review').isHidden(),true);assert.equal(await page.locator('#delivery-copies').inputValue(),'2');assert.equal(await page.locator('#delivery-color').inputValue(),color);assert.equal(await page.locator('#delivery-pages').inputValue(),'2-3');await page.locator('#delivery-check').click();await wait(()=>page.locator('#delivery-review').isVisible(),'review after printer refresh');await approve();const previousGrant=await page.locator('#delivery-approval').inputValue();
 await page.locator('#delivery-copies').fill('3');assert.equal(await page.locator('#delivery-review').isHidden(),true);assert.equal(await page.locator('#delivery-approval').inputValue(),'');assert.equal(await page.locator('#delivery-send').isDisabled(),true);
 await page.locator('#delivery-check').click();await wait(()=>page.locator('#delivery-review').isVisible(),'changed conditions rechecked');await approve();assert.notEqual(await page.locator('#delivery-approval').inputValue(),previousGrant);
 if(screenshots){await mkdir(screenshots,{recursive:true});await page.screenshot({path:`${screenshots}/delivery-desktop.png`,fullPage:true});await page.locator('#delivery-panel').screenshot({path:`${screenshots}/delivery-panel-desktop.png`});}
 for(const lang of ['ja','zh-CN','en']){
  await page.locator('#language').selectOption(lang);
  assert.equal(await page.locator('#delivery-copies').inputValue(),'3');assert.equal(await page.locator('#delivery-color').inputValue(),color);
  for(const width of [320,375,430]){await page.setViewportSize({width,height:900});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`${lang}/${width} no page overflow`);if(screenshots&&width===375){await page.screenshot({path:`${screenshots}/delivery-mobile-${lang}.png`,fullPage:true});await page.locator('#delivery-panel').screenshot({path:`${screenshots}/delivery-panel-mobile-${lang}.png`});}}
 }
 await page.setViewportSize({width:1440,height:1000});await page.locator('#delivery-send').click();
 await wait(()=>page.locator('#delivery-status').innerText().then(v=>v.includes('Device accepted')),'device acceptance is separate from raster export');
 assert.equal(writes.length,1);const first=writes[0];assert.equal(first.copies,3);assert.equal(first.colorAttemptId,color);assert.equal(first.sides,'two-sided-long-edge');assert.deepEqual(first.pageRanges,[{first:2,last:3}]);assert.match(first.expectedTicketSha256,/^[0-9a-f]{64}$/);
 const record=await api(`/api/v1/deliveries/${first.requestId}`);assert.equal(record.ticket.colorAttemptId,color);assert.equal(record.ticket.copies,3);
 await page.reload();assert.equal(await page.locator('#submit').isDisabled(),true);await connect(token);await openOutput();
 await wait(()=>page.locator(`#delivery-history option[value="${first.requestId}"]`).count().then(n=>n===1),'delivery recovered after reload');
 await page.locator('#delivery-history').selectOption(first.requestId);await wait(()=>page.locator('#delivery-cancel').isEnabled(),'current delivery revision loaded');
 const observe=async(state,reasons,progressMode=0)=>{const response=await fetch(new URL('/__test/device',url),{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({state,reasons,progressMode})});assert.equal(response.status,204);};
 await observe(5,['job-printing']);await wait(()=>page.locator('#delivery-status').innerText().then(v=>v.includes('Printing')),'actual device printing state');
 assert.equal((await api(`/api/v1/deliveries/${first.requestId}`)).deviceObservation.state,5);
 const counter=async(label)=>page.locator('#delivery-status').evaluate((root,label)=>Array.from(root.querySelectorAll('dt')).filter(n=>n.textContent===label).map(n=>n.nextElementSibling.textContent),label);
 assert.deepEqual(await counter('Completed impressions'),['3']);assert.deepEqual(await counter('Completed sheets'),['2']);
 await observe(5,['job-printing'],1);await wait(()=>counter('Completed impressions').then(v=>v.length===0),'unsupported counter is omitted');assert.deepEqual(await counter('Completed sheets'),[]);
 await observe(5,['job-printing'],2);await wait(()=>counter('Completed impressions').then(v=>v[0]==='0'),'reported zero is visible');assert.deepEqual(await counter('Completed sheets'),['0']);
 await observe(5,['job-printing'],3);await wait(()=>counter('Completed impressions').then(v=>v[0]==='3'),'independent impressions counter');assert.deepEqual(await counter('Completed sheets'),[]);

 const previousObservation=(await api(`/api/v1/deliveries/${first.requestId}`)).deviceObservation;
 await observe(5,['job-printing'],9);await wait(()=>page.locator('#delivery-status').innerText().then(v=>v.includes('Invalid device response')),'failed query is visible');
 assert((await page.locator('#delivery-status').innerText()).includes('Last reported device state'));
 assert.deepEqual((await api(`/api/v1/deliveries/${first.requestId}`)).deviceObservation,previousObservation);
 assert.equal(writes.length,1,'failed query never resends');
 if(screenshots)await page.locator('#delivery-panel').screenshot({path:`${screenshots}/device-query-failed-desktop.png`});
 await observe(6,['printer-stopped','media-needed','vendor-needs-attention']);
 await wait(()=>page.locator('#delivery-status').innerText().then(v=>v.includes('Device stopped')&&v.includes('Paper needed')),'stopped state and device reasons');
 assert.equal(writes.length,1,'device observations never resend');
 if(screenshots)await page.locator('#delivery-panel').screenshot({path:`${screenshots}/device-stopped-desktop.png`});
 for(const [lang,label] of [['ja','機器停止中'],['zh-CN','设备已停止'],['en','Device stopped']]){
  await page.locator('#language').selectOption(lang);await page.setViewportSize({width:375,height:1000});
  await wait(()=>page.locator('#delivery-status').innerText().then(v=>v.includes(label)),'localized device status');
  assert((await page.locator('#delivery-status').innerText()).includes('vendor-needs-attention'));
  const labels={ja:['完了印刷面数','完了用紙枚数'],'zh-CN':['已完成印面数','已完成纸张数'],en:['Completed impressions','Completed sheets']}[lang];
  assert.deepEqual(await counter(labels[0]),['4']);assert.deepEqual(await counter(labels[1]),['2']);
  await observe(6,['printer-stopped','media-needed','vendor-needs-attention'],9);
  const failedLabel={ja:'機器の応答が不正です',en:'Invalid device response','zh-CN':'设备响应无效'}[lang];
  await wait(()=>page.locator('#delivery-status').innerText().then(v=>v.includes(failedLabel)),'localized failed query');
  if(screenshots&&lang==='ja')await page.locator('#delivery-panel').screenshot({path:`${screenshots}/device-query-failed-mobile-ja.png`});
  await observe(6,['printer-stopped','media-needed','vendor-needs-attention']);
  await wait(()=>page.locator('#delivery-status').innerText().then(v=>!v.includes(failedLabel)),'query recovery is visible');

  assert((await page.locator('[data-title=deliveryHelp]').getAttribute('title')).includes({ja:'原稿ページの進捗',en:'source-page progress','zh-CN':'原稿页面进度'}[lang]));

  for(const width of [320,375,430]){await page.setViewportSize({width,height:1000});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'device counters and notices fit mobile');}await page.setViewportSize({width:375,height:1000});
  if(screenshots&&lang==='ja')await page.locator('#delivery-panel').screenshot({path:`${screenshots}/device-stopped-mobile-ja.png`});
 }
 await page.locator('#language').selectOption('en');await page.setViewportSize({width:1440,height:1000});
 await observe(5,['job-printing']);await wait(()=>page.locator('#delivery-status').innerText().then(v=>v.includes('Printing')&&!v.includes('Device stopped')),'device resumed without requeue');
 await page.locator('#delivery-cancel').click();await wait(()=>page.locator('#delivery-status').innerText().then(v=>v.includes('Cancelled')),'device cancellation observed');assert.equal(writes.length,1);
 // Lose the response or receive a gateway failure after queueing. Recovery must only GET the same ID.
 for(const failure of ['abort','502']) {
  await configure('1','3','one-sided');await approve();
  const route=new URL(`/api/v1/jobs/${job.id}/deliveries`,url).href;
  await page.route(route,async handler=>{const response=await handler.fetch();assert.equal(response.status(),202);if(failure==='abort')await handler.abort('failed');else await handler.fulfill({status:502,contentType:'application/json',body:JSON.stringify({error:{code:'simulated_gateway_failure'}})});});
  const count=writes.length;await page.locator('#delivery-send').click();await wait(()=>writes.length===count+1,'one new output request');const second=writes[count];
  await wait(()=>page.locator('#delivery-status').innerText().then(v=>v.includes(second.requestId)),'uncertain response recovered from persistent ID');
  await page.waitForTimeout(500);assert.equal(writes.length,count+1);assert.equal((await api(`/api/v1/jobs/${job.id}/deliveries`)).deliveries.length,count+1);
  await page.unroute(route);
 }
 await page.locator('#delivery-reload').click();await wait(()=>page.locator('#delivery-reload').isEnabled(),'history refresh complete');
 await page.waitForTimeout(3500);
 const expectedHistory=(await api(`/api/v1/jobs/${job.id}/deliveries`)).deliveries.map(record=>record.id);
 assert.deepEqual(await page.locator('#delivery-history option').evaluateAll(options=>options.map(option=>option.value).filter(Boolean)),expectedHistory,'polling retains creation order and latest revisions');
 await page.locator('#connection').click();await connect('reader');await openOutput();await configure('1','3','one-sided');
 assert.equal(await page.locator('#delivery-send').isDisabled(),true);assert.equal(await page.locator('#delivery-issuer').isHidden(),true);assert.equal(writes.length,3);
 assert.deepEqual(errors,[]);
 console.log(`PASS ${url}: real color output, live media/copies/duplex, structured printer notices, independent PWG/CUPS telemetry, zero/unknown/units/expiry/query-failure/text safety, non-queueing review, exact approval, invalidation, device state/cancel, reload recovery, lost-response recovery without repost, read-only scopes, EN/JA/zh 320/375/430px; Browser skill unavailable, Playwright with installed Chrome`);
}catch(error){console.error('Output UI state',await page.evaluate(()=>({notice:document.getElementById('notice').textContent,review:document.getElementById('delivery-summary').textContent,status:document.getElementById('delivery-status').textContent,printer:document.getElementById('delivery-printer').value})));if(process.env.JRIP_UI_SCREENSHOTS){await mkdir(process.env.JRIP_UI_SCREENSHOTS,{recursive:true});await page.screenshot({path:`${process.env.JRIP_UI_SCREENSHOTS}/delivery-failure.png`,fullPage:true});}throw error;}finally{await browser.close();}
