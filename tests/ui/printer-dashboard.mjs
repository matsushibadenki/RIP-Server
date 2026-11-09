import {chromium} from 'playwright';
import assert from 'node:assert/strict';
import {mkdir} from 'node:fs/promises';
const url=process.env.JRIP_UI_URL;assert(url);
const browser=await chromium.launch({headless:true,executablePath:process.env.JRIP_CHROMIUM_PATH||undefined});
const context=await browser.newContext({viewport:{width:1440,height:1000},locale:'en-US'});const page=await context.newPage();const errors=[];const writes=[];
page.on('pageerror',e=>errors.push(e.message));page.on('console',m=>{if(['warning','error'].includes(m.type())&&!/Failed to load resource.*(400|503)/.test(m.text()))errors.push(m.text());});
page.on('request',r=>{if(r.method()!=='GET')writes.push(r.url());});
const wait=async f=>{for(let i=0;i<200;i++){if(await f())return;await page.waitForTimeout(50);}throw Error('state timeout');};
const queries=async()=> (await (await context.request.get(new URL('/__test/status',url).href)).json()).queries;
const connect=async()=>{await page.locator('#connection').click();await page.locator('#token').fill('reader');await page.locator('#login-form button[type=submit]').click();await wait(()=>page.locator('#login').isVisible().then(v=>!v));};
const select=async id=>{await page.locator('#dashboard-printer').selectOption(id);await wait(()=>page.locator('#dashboard-printer-refresh').isEnabled().then(v=>id?v:true));};
try{
 await page.goto(url);assert.equal(await page.title(),'J-RIP');assert.equal(new URL(page.url()).pathname,'/console');assert.equal(await page.locator('#printer-dashboard').isHidden(),true);
 await connect();assert.equal(await page.locator('#delivery-panel').isHidden(),true,'no completed job');
 await page.locator('#printer-dashboard > summary').click();await wait(()=>page.locator('#dashboard-printer option').count().then(n=>n===3));
 assert.equal(await page.locator('header').count(),1);const initial=await queries();await select('p1');
 await wait(()=>page.locator('#dashboard-printer-live').getAttribute('data-source').then(v=>v==='stored'));
 assert((await page.locator('#dashboard-printer-conditions').innerText()).includes('Paper jam'));
 assert.equal(await queries(),initial,'open/select only reads stored evidence');
 for(const [lang,label,notice] of [['en','Saved report','Paper jam'],['ja','保存済みの報告','紙詰まり'],['zh-CN','已保存报告','卡纸']]){
  await page.locator('#language').selectOption(lang);assert((await page.locator('#dashboard-printer-query-state').innerText()).includes(label));assert((await page.locator('#dashboard-printer-conditions').innerText()).includes(notice));
  for(const width of [320,375,430,1440]){await page.setViewportSize({width,height:1000});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`${lang}/${width}`);}
 }
 const screenshots=process.env.JRIP_UI_SCREENSHOTS;
 if(screenshots){await mkdir(screenshots,{recursive:true});await page.locator('#printer-dashboard').screenshot({path:`${screenshots}/dashboard-panel-desktop.png`});await page.setViewportSize({width:375,height:1000});await page.locator('#printer-dashboard').screenshot({path:`${screenshots}/dashboard-panel-mobile.png`});}
 await page.locator('#language').selectOption('en');await page.setViewportSize({width:1440,height:1000});
 await select('p2');assert.equal(await page.locator('#dashboard-printer-snapshot-content').isHidden(),true);assert.equal(await queries(),initial);
 await select('p1');await page.locator('#dashboard-printer-refresh').click();await wait(()=>page.locator('#dashboard-printer-refresh').isEnabled());assert.equal(await page.locator('#dashboard-printer-live').getAttribute('data-source'),'live');assert.equal(await queries(),initial+1);
 await context.request.post(new URL('/__test/fail',url).href,{data:true});await page.locator('#dashboard-printer-refresh').click();await wait(()=>page.locator('#dashboard-printer-refresh').isEnabled());assert((await page.locator('#dashboard-printer-query-state').innerText()).includes('Query unavailable'));assert((await page.locator('#dashboard-printer-conditions').innerText()).includes('Paper jam'));
 // Delay an old device read: a later selection must win.
 const route=new URL('/api/v1/printers/p1/observations?limit=1',url).href;let release;const gate=new Promise(r=>release=r);let entered=false;
 await page.route(route,async h=>{const response=await h.fetch();entered=true;await gate;await h.fulfill({response});});
 await page.locator('#dashboard-printer').selectOption('p1');await wait(()=>entered);await select('p2');release();await page.waitForTimeout(150);assert.equal(await page.locator('#dashboard-printer-snapshot-content').isHidden(),true,'late old device response is ignored');await page.unroute(route);
 const listRoute=new URL('/api/v1/printers',url).href;await page.route(listRoute,h=>h.fulfill({status:200,contentType:'application/json',body:JSON.stringify({printers:[]})}));await page.locator('#dashboard-list-refresh').click();await wait(()=>page.locator('#dashboard-list-refresh').isEnabled());assert.equal(await page.locator('#dashboard-printer option').count(),1);assert.equal(await page.locator('#dashboard-printer-live').isHidden(),true,'removed registrations clear stored state');await page.unroute(listRoute);await page.locator('#dashboard-list-refresh').click();await wait(()=>page.locator('#dashboard-printer option').count().then(n=>n===3));
 await select('p1');await page.locator('#connection').click();assert.equal(await page.locator('#printer-dashboard').isHidden(),true);assert.equal(await page.locator('#dashboard-printer-conditions').textContent(),'');
 await connect();await page.locator('#printer-dashboard > summary').click();await wait(()=>page.locator('#dashboard-printer option').count().then(n=>n===3));assert.equal(await page.locator('#dashboard-printer').inputValue(),'','new session does not inherit selection');
 assert.deepEqual(writes,[],'dashboard never sends state-changing requests');assert.deepEqual(errors,[]);
 console.log(`PASS ${url}: no-job read-only dashboard, saved state after restart, independent device selection, explicit live refresh, failed query, late response fencing, session reset, EN/JA/zh, 320/375/430/1440px; Browser skill unavailable, Playwright with installed Chrome`);
}finally{await browser.close();}
