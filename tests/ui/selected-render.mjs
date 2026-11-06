// Browser -> real REST upload -> bounded worker/Docker -> original source-page preview.
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readFile, mkdir } from 'node:fs/promises';
const {JRIP_UI_URL:url,JRIP_UI_TOKEN:token,JRIP_UI_PDF:pdf,JRIP_UI_ROOT:root,JRIP_UI_WORKER_BIN:worker}=process.env;
assert(url&&token&&pdf&&root&&worker,'Use the Rust console_browser selected-render integration test');
const browser=await chromium.launch({headless:true,executablePath:process.env.JRIP_CHROMIUM_PATH||undefined});
const context=await browser.newContext({viewport:{width:1440,height:1000},locale:'ja-JP'});
const page=await context.newPage();const errors=[];
page.on('pageerror',error=>errors.push(error.message));
page.on('console',message=>{if(['error','warning'].includes(message.type()))errors.push(message.text());});
const waitFor=async(fn,label)=>{for(let n=0;n<200;n++){if(await fn())return;await page.waitForTimeout(100);}throw new Error(label);};
try {
 await page.goto(url);assert.equal(await page.title(),'J-RIP');assert.equal(new URL(page.url()).pathname,'/console');
 await page.locator('#connection').click();await page.locator('#token').fill(token);await page.locator('#login-form button[type=submit]').click();
 await waitFor(()=>page.locator('#login').isVisible().then(value=>!value),'connect');
 for(const engine of ['auto','ghostscript']) {
 await page.locator('#file').setInputFiles({name:`real-${engine}.pdf`,mimeType:'application/pdf',buffer:await readFile(pdf)});
 await page.locator('#engine').selectOption(engine);
 await page.locator('#page-ranges').fill('２');await page.locator('#color').selectOption('rgb');await page.locator('#dpi').fill('300');
 await page.locator('#submit').click();await waitFor(()=>page.locator('#selected-name').innerText().then(value=>value===`real-${engine}.pdf`),'submitted');
 const api=async path=>{const response=await context.request.get(new URL(path,url).href,{headers:{Authorization:`Bearer ${token}`}});assert.equal(response.status(),200);return response.json();};
 const job=(await api('/api/v1/jobs')).find(job=>job.manifest.name===`real-${engine}.pdf`);
 assert(job);assert.equal(await page.locator('#preview-image').isHidden(),true);
 assert.deepEqual(job.manifest.page_ranges,[{first:2,last:2}]);
 await promisify(execFile)(worker,[job.id],{env:{...process.env,JRIP_DATA_ROOT:root},timeout:90000,maxBuffer:1024*1024});
 await page.locator('#refresh').click();
 await waitFor(()=>page.locator('#preview-image').evaluate(image=>!image.hidden&&image.complete&&image.naturalWidth>0),'rendered preview');
 assert.equal(await page.locator('#page-label').innerText(),'原稿ページ');assert.deepEqual(await page.locator('#page option').allTextContents(),['2']);
 assert.equal(await page.locator('#page').inputValue(),'page-000001.tiff');
 const sample=await page.locator('#preview-image').evaluate(image=>{
  const canvas=document.createElement('canvas');canvas.width=image.naturalWidth;canvas.height=image.naturalHeight;
  const context=canvas.getContext('2d');context.drawImage(image,0,0);return Array.from(context.getImageData(Math.floor(canvas.width/2),Math.floor(canvas.height/2),1,1).data);
 });
 // Ghostscript's embedded ICC can introduce one 8-bit rounding step in sRGB preview.
 assert(sample.slice(0,3).every((value,index)=>Math.abs(value-[0,255,0][index])<=2),`preview must show green source page 2, not red source page 1: ${sample}`);
 assert.equal(sample[3],255);
 assert.equal((await api(`/api/v1/jobs/${job.id}`)).selected_engine,engine==='auto'?'mupdf':'ghostscript');
 const artifacts=await api(`/api/v1/jobs/${job.id}/artifacts`);assert.equal(artifacts.length,1);assert.equal(artifacts[0].sourcePage,2);assert.equal(artifacts[0].sourcePageCount,2);
 const validation=await api(`/api/v1/jobs/${job.id}/source-validation`);assert.equal(validation.pageCount,2);assert.equal(validation.strictPdfInspection,true);
 const history=await api(`/api/v1/jobs/${job.id}/events`);const progress=history.events.filter(event=>event.progressPhase==='rendering');assert.equal(progress.length,1);assert.equal(progress[0].totalPages,1);
 }
 assert.deepEqual(errors,[]);
 if(process.env.JRIP_UI_SCREENSHOTS){await mkdir(process.env.JRIP_UI_SCREENSHOTS,{recursive:true});await page.screenshot({path:`${process.env.JRIP_UI_SCREENSHOTS}/real-render.png`,fullPage:true});}
 console.log(`PASS: ${url} -> real PDF range upload -> MuPDF/Ghostscript workers -> green original-page-2 BMP preview, source count 2/render count 1; no browser warnings/errors`);
} finally {await browser.close();}
