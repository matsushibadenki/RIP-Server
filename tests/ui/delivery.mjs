import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import {mkdir} from 'node:fs/promises';
const {JRIP_UI_URL:url,JRIP_UI_TOKEN:token,JRIP_UI_COLOR:color}=process.env;
assert(url&&token&&color,'Use the isolated Rust delivery browser test');
const browser=await chromium.launch({headless:true,executablePath:process.env.JRIP_CHROMIUM_PATH||undefined});
const context=await browser.newContext({viewport:{width:1440,height:1000},locale:'en-US'});
const page=await context.newPage();const errors=[];const writes=[];
page.on('pageerror',error=>errors.push(error.message));
page.on('console',message=>{if(['warning','error'].includes(message.type())&&!/Failed to load resource: (net::ERR_FAILED|the server responded with a status of 502)/.test(message.text()))errors.push(message.text());});
page.on('request',request=>{if(request.method()==='POST'&&new URL(request.url()).pathname.endsWith('/deliveries'))writes.push(JSON.parse(request.postData()));});
const wait=async(fn,label)=>{for(let n=0;n<200;n++){if(await fn())return;await page.waitForTimeout(100);}throw new Error(label);};
const api=async path=>{const response=await context.request.get(new URL(path,url).href,{headers:{Authorization:`Bearer ${token}`}});assert.equal(response.status(),200);return response.json();};
async function connect(value){await page.locator('#connection').click();await page.locator('#token').fill(value);await page.locator('#login-form button[type=submit]').click();await wait(()=>page.locator('#login').isVisible().then(v=>!v),'connected');}
async function openOutput(){await page.getByRole('button',{name:'test.pdf',exact:true}).click();await wait(()=>page.locator('#delivery-panel').isVisible(),'physical controls visible');await page.locator('#delivery-panel summary').click();await wait(()=>page.locator('#delivery-printer').isEnabled(),'destinations loaded');}
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
 await page.goto(url);assert.equal(await page.title(),'J-RIP');assert.equal(new URL(page.url()).pathname,'/console');
 await connect(token);await openOutput();await configure('2');
 const job=(await api('/api/v1/jobs')).find(job=>job.manifest.name==='test.pdf');assert(job);
 assert.equal(writes.length,0);assert.deepEqual((await api(`/api/v1/jobs/${job.id}/deliveries`)).deliveries,[]);
 await page.locator('#delivery-send').click();await wait(()=>page.locator('#notice').innerText().then(v=>v.includes('approval')),'missing approval blocked');assert.equal(writes.length,0);
 await approve();const previousGrant=await page.locator('#delivery-approval').inputValue();
 await page.locator('#delivery-copies').fill('3');assert.equal(await page.locator('#delivery-review').isHidden(),true);assert.equal(await page.locator('#delivery-approval').inputValue(),'');assert.equal(await page.locator('#delivery-send').isDisabled(),true);
 await page.locator('#delivery-check').click();await wait(()=>page.locator('#delivery-review').isVisible(),'changed conditions rechecked');await approve();assert.notEqual(await page.locator('#delivery-approval').inputValue(),previousGrant);
 const screenshots=process.env.JRIP_UI_SCREENSHOTS;if(screenshots){await mkdir(screenshots,{recursive:true});await page.screenshot({path:`${screenshots}/delivery-desktop.png`,fullPage:true});await page.locator('#delivery-panel').screenshot({path:`${screenshots}/delivery-panel-desktop.png`});}
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
 console.log(`PASS ${url}: real color output, live media/copies/duplex, non-queueing review, exact approval, invalidation, device state/cancel, reload recovery, lost-response recovery without repost, read-only scopes, EN/JA/zh 320/375/430px; Browser skill unavailable, Playwright with installed Chrome`);
}catch(error){console.error('Output UI state',await page.evaluate(()=>({notice:document.getElementById('notice').textContent,review:document.getElementById('delivery-summary').textContent,status:document.getElementById('delivery-status').textContent,printer:document.getElementById('delivery-printer').value})));if(process.env.JRIP_UI_SCREENSHOTS){await mkdir(process.env.JRIP_UI_SCREENSHOTS,{recursive:true});await page.screenshot({path:`${process.env.JRIP_UI_SCREENSHOTS}/delivery-failure.png`,fullPage:true});}throw error;}finally{await browser.close();}
