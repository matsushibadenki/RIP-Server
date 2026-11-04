import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import {mkdir} from 'node:fs/promises';
const url=process.env.JRIP_UI_URL;
assert(url,'Use the Rust console_browser integration test or set JRIP_UI_URL to an isolated test server');
const token=process.env.JRIP_UI_TOKEN;
assert(token,'JRIP_UI_TOKEN required');
const browser=await chromium.launch({headless:true,executablePath:process.env.JRIP_CHROMIUM_PATH||undefined});
const context=await browser.newContext({viewport:{width:1440,height:900},locale:'en-US'});
const page=await context.newPage();
const errors=[];page.on('pageerror',error=>errors.push(error.message));
const sent=[];page.on('request',r=>{if(r.url().includes('/api/'))sent.push({url:r.url(),method:r.method(),headers:r.headers()});});
const expect=async(fn,message)=>{for(let i=0;i<100;i++){if(await fn())return;await page.waitForTimeout(100);}throw new Error(message);};
async function connect(value){await page.locator('#connection').click();await page.locator('#token').fill(value);await page.locator('#login-form button[type=submit]').click();}
try{
 await page.goto(url);assert.equal(await page.title(),'J-RIP');
 assert.equal(await page.locator('#submit').isDisabled(),true);
 await connect('invalid-test-token');await expect(async()=>!(await page.locator('#login-form button[type=submit]').isDisabled()),'login completed');
 assert.match(await page.locator('#notice').innerText(),/expired/i);
 await page.locator('#close-login').click();await connect(token);
 await expect(()=>page.locator('#login').isVisible().then(v=>!v),'connected');
 assert.equal(await page.locator('#token').inputValue(),'');
 await page.getByRole('button',{name:'ready-proof.pdf',exact:true}).click();
 await expect(()=>page.locator('#preview-image').evaluate(img=>img.complete&&img.naturalWidth===240),'first preview decoded');
 await page.locator('#page').selectOption('page-000002.tiff');
 await expect(()=>page.locator('#preview-image').evaluate(img=>img.complete&&img.naturalWidth===240),'second preview decoded');
 assert.equal(await page.locator('#cancel').isDisabled(),true);
 await page.locator('#dpi').fill('600');await page.locator('#color').selectOption('gray');await page.locator('#engine').selectOption('ghostscript');await page.locator('#priority').fill('73');
 const chooserPromise=page.waitForEvent('filechooser');await page.locator('#drop').click();const chooser=await chooserPromise;
 await chooser.setFiles({name:'layout-proof.pdf',mimeType:'application/pdf',buffer:Buffer.from('%PDF-1.4\n')});
 await page.locator('#submit').click();
 await expect(()=>page.locator('#selected-name').innerText().then(v=>v==='layout-proof.pdf'),'uploaded');
 const list=await (await context.request.get(new URL('/api/v1/jobs',url).href,{headers:{Authorization:`Bearer ${token}`}})).json();
 const job=list.find(j=>j.manifest.name==='layout-proof.pdf');assert(job);
 assert.equal(job.manifest.dpi,600);assert.equal(job.manifest.color_mode,'gray');assert.equal(job.manifest.engine,'ghostscript');assert.equal(job.manifest.priority,73);
 await page.locator('#hold').click();await expect(()=>page.locator('#metadata').innerText().then(v=>v.includes('Held')),'held');
 await page.locator('#hold').click();await expect(()=>page.locator('#metadata').innerText().then(v=>v.includes('Queued')),'released');
 await page.locator('#cancel').click();await expect(()=>page.locator('#metadata').innerText().then(v=>v.includes('Cancelled')),'cancelled');
 const controls=sent.filter(r=>r.method==='POST'&&/\/(hold|release|cancel)$/.test(r.url));assert.equal(controls.length,3);assert(controls.every(r=>/^\d+$/.test(r.headers['x-jrip-expected-revision'])));
 assert.equal(await page.locator('#cancel').isDisabled(),true);
 // Local validation must not submit unsupported bytes.
 const before=sent.filter(r=>r.method==='POST'&&new URL(r.url).pathname==='/api/v1/jobs').length;
 await page.locator('#file').setInputFiles({name:'bad.pdf',mimeType:'application/pdf',buffer:Buffer.from('invalid')});await page.locator('#submit').click();
 await expect(()=>page.locator('#notice').innerText().then(v=>v.includes('32 MiB')),'invalid source blocked');
 assert.equal(sent.filter(r=>r.method==='POST'&&new URL(r.url).pathname==='/api/v1/jobs').length,before);
 // Same-origin UI, text-safe names and both PostScript formats.
 for(const [name,source,format] of [['<img src=x onerror=alert(1)>.ps','%!PS\n','application/postscript'],['sample.eps','%!PS-Adobe-3.0 EPSF-3.0\n','application/eps']]){
  await page.locator('#file').setInputFiles({name,mimeType:'application/postscript',buffer:Buffer.from(source)});await page.locator('#submit').click();
  await expect(()=>page.locator('#selected-name').innerText().then(v=>v===name),'PS/EPS upload');
  const jobs=await (await context.request.get(new URL('/api/v1/jobs',url).href,{headers:{Authorization:`Bearer ${token}`}})).json();assert.equal(jobs.find(j=>j.manifest.name===name).manifest.format,format);
 }
 assert.equal(await page.locator('#selected-name img').count(),0);
 const latest=await (await context.request.get(new URL('/api/v1/jobs',url).href,{headers:{Authorization:`Bearer ${token}`}})).json();
 const pollingJob=latest.find(j=>j.manifest.name==='sample.eps');
 const heldResponse=await context.request.post(new URL(`/api/v1/jobs/${pollingJob.id}/hold`,url).href,{headers:{Authorization:`Bearer ${token}`,'x-jrip-expected-revision':String(pollingJob.revision)}});assert.equal(heldResponse.status(),200);
 await expect(()=>page.locator('#metadata').innerText().then(v=>v.includes('Held')),'external state update appears through polling');
 // Server rejects stale controls; console refreshes the selected revision.
 const cancelPath=new URL(`/api/v1/jobs/${pollingJob.id}/cancel`,url).href;
 await page.route(cancelPath,async route=>{
  const response=await context.request.post(new URL(`/api/v1/jobs/${pollingJob.id}/release`,url).href,{headers:{Authorization:`Bearer ${token}`,'x-jrip-expected-revision':String(pollingJob.revision+1)}});assert.equal(response.status(),200);
  await route.continue();
 });
 await page.locator('#cancel').click();
 await expect(()=>page.locator('#notice').innerText().then(v=>v.includes('409')),'stale cancellation rejected');
 await expect(()=>page.locator('#metadata').innerText().then(v=>v.includes('Queued')),'revision conflict refreshed');


 // Desktop visual reference with a completed preview selected.
 await page.getByRole('button',{name:'ready-proof.pdf',exact:true}).click();await expect(()=>page.locator('#preview-image').evaluate(img=>img.complete&&img.naturalWidth>0),'preview');
 const screenshots=process.env.JRIP_UI_SCREENSHOTS;
 if(screenshots){await page.setViewportSize({width:1487,height:1058});await mkdir(screenshots,{recursive:true});await page.screenshot({path:`${screenshots}/desktop.png`,fullPage:true});}
 for(const lang of ['ja','zh-CN','en']){
  await page.locator('#language').selectOption(lang);assert.equal(await page.locator('html').getAttribute('lang'),lang);
  for(const width of [375,430]){
   await page.setViewportSize({width,height:900});
   assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`no page overflow ${lang}/${width}`);
   assert(await page.locator('#submit').isVisible());
   if(screenshots&&width===375)await page.screenshot({path:`${screenshots}/mobile-${lang}.png`,fullPage:true});
  }
 }
 if(screenshots){await page.screenshot({path:`${screenshots}/mobile.png`,fullPage:true});}
 const storage=await page.evaluate(()=>({local:localStorage.length,session:sessionStorage.length,cookie:document.cookie}));assert.deepEqual(storage,{local:0,session:0,cookie:''});
 assert(sent.every(r=>!r.url.includes(token)));
 await page.locator('#connection').click();assert.equal(await page.locator('#jobs tr').count(),0);assert.equal(await page.locator('#submit').isDisabled(),true);assert.equal(await page.locator('#preview-image').isHidden(),true);
 await connect('console-readonly-test-token');await expect(()=>page.locator('#login').isVisible().then(v=>!v),'readonly login');
 await page.locator('#file').setInputFiles({name:'denied.pdf',mimeType:'application/pdf',buffer:Buffer.from('%PDF-1.4\n')});await page.locator('#submit').click();
 await expect(()=>page.locator('#notice').innerText().then(v=>v.includes('403')),'scope denied');
 await page.reload();assert.equal(await page.locator('#submit').isDisabled(),true);assert.equal(await page.locator('#jobs tr').count(),0);
 assert.deepEqual(errors,[]);
 console.log('PASS: real upload PDF/PS/EPS, settings, polling UI, hold/release/cancel revisions, preview pages, scopes, disconnect/reload, text safety, EN/JA/zh and mobile overflow');
}finally{await browser.close();}
