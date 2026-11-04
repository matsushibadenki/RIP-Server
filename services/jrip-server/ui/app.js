'use strict';
const $ = id => document.getElementById(id);
const texts = {
 en:{connect:'Connect',disconnect:'Disconnect',newJob:'New job',file:'File',choose:'Choose a file or drop it here',resolution:'Resolution · dpi',color:'Color',gray:'Gray',engine:'Engine',automatic:'Automatic',priority:'Priority',submit:'Submit',jobs:'Jobs',name:'Name',state:'State',updated:'Updated',refresh:'Refresh',previous:'Previous',next:'Next',preview:'Preview',previewHelp:'Screen preview, not a color proof. CMYK requires a matching embedded ICC. Completed means raster export, not physical printing.',selectJob:'Select a job',page:'Page',hold:'Hold',release:'Release',cancel:'Cancel',close:'Close',tokenHelp:'Token stays in memory only. Disconnect or reload to clear it.',disconnected:'Connect to view jobs',empty:'No jobs',loading:'Loading…',noPreview:'Preview is available after raster export',unsupported:'This page cannot be previewed. Check its ICC and raster format.',fileInvalid:'Choose a PDF, PS or EPS document, up to 32 MiB.',ambiguous:'Submission was not confirmed. Refresh the queue before submitting again.',submitted:'Job submitted',cancelled:'Job cancelled',held:'Job held',released:'Job released',rejected:'The server rejected this request. Check settings.',network:'Connection failed. Check the server and refresh.',expired:'Session expired. Connect again.',denied:'Your credential does not allow this operation.',conflict:'Job changed. Refresh and try again.',id:'ID',pages:'Pages',error:'Error',unknown:'—',QUEUED:'Queued',RIPPING:'Rendering',COMPLETED:'Raster exported',CANCELLED:'Cancelled',FAILED:'Failed',HELD:'Held',RECEIVED:'Received',VALIDATING:'Validating',PREFLIGHT:'Preflight',SPOOLING:'Spooling',COLOR_PROCESSING:'Color processing',HALFTONING:'Halftoning',PRINTING:'Printing'},
 ja:{connect:'接続',disconnect:'切断',newJob:'原稿を投入',file:'原稿',choose:'原稿を選択・ドロップ',resolution:'解像度 · dpi',color:'色空間',gray:'グレー',engine:'エンジン',automatic:'自動',priority:'優先度',submit:'投入',jobs:'ジョブ',name:'原稿名',state:'状態',updated:'更新',refresh:'更新',previous:'前へ',next:'次へ',preview:'プレビュー',previewHelp:'配置確認用です。色校正ではありません。CMYKには一致する埋め込みICCが必要です。完了はラスター出力を表し、実機印刷の完了ではありません。',selectJob:'ジョブを選択',page:'ページ',hold:'保留',release:'再開',cancel:'取消',close:'閉じる',tokenHelp:'トークンはメモリ内だけに保持します。切断・再読み込みで消去します。',disconnected:'接続するとジョブを表示します',empty:'ジョブはありません',loading:'読み込み中…',noPreview:'ラスター出力後に表示できます',unsupported:'このページは表示できません。ICCとラスター形式を確認してください。',fileInvalid:'32 MiB以下のPDF・PS・EPS原稿を選択してください。',ambiguous:'投入の完了を確認できませんでした。キューを更新してから再投入してください。',submitted:'投入しました',cancelled:'取り消しました',held:'保留しました',released:'再開しました',rejected:'要求が拒否されました。設定を確認してください。',network:'接続できません。サーバーを確認して更新してください。',expired:'認証が失効しました。再接続してください。',denied:'この操作の権限がありません。',conflict:'ジョブが更新されています。再確認してください。',id:'ID',pages:'ページ数',error:'エラー',unknown:'—',QUEUED:'待機中',RIPPING:'描画中',COMPLETED:'ラスター出力済み',CANCELLED:'取消済み',FAILED:'失敗',HELD:'保留中',RECEIVED:'受付済み',VALIDATING:'検証中',PREFLIGHT:'プリフライト',SPOOLING:'保存中',COLOR_PROCESSING:'色変換中',HALFTONING:'網点処理中',PRINTING:'印刷中'},
 'zh-CN':{connect:'连接',disconnect:'断开',newJob:'提交原稿',file:'原稿',choose:'选择或拖入原稿',resolution:'分辨率 · dpi',color:'色彩空间',gray:'灰度',engine:'引擎',automatic:'自动',priority:'优先级',submit:'提交',jobs:'任务',name:'原稿名',state:'状态',updated:'更新',refresh:'刷新',previous:'上一页',next:'下一页',preview:'预览',previewHelp:'仅供版面检查，不用于色彩校样。CMYK需要匹配的嵌入ICC。完成表示光栅导出，不表示实机打印完成。',selectJob:'选择任务',page:'页码',hold:'暂停',release:'恢复',cancel:'取消',close:'关闭',tokenHelp:'令牌仅保存在内存中。断开或刷新页面后清除。',disconnected:'连接后显示任务',empty:'没有任务',loading:'加载中…',noPreview:'光栅导出后可预览',unsupported:'无法预览此页，请检查ICC及光栅格式。',fileInvalid:'请选择不超过32 MiB的PDF、PS或EPS原稿。',ambiguous:'未确认提交结果，请刷新队列后再提交。',submitted:'已提交',cancelled:'已取消',held:'已暂停',released:'已恢复',rejected:'请求被拒绝，请检查设置。',network:'连接失败，请检查服务器并刷新。',expired:'认证已失效，请重新连接。',denied:'没有执行此操作的权限。',conflict:'任务已更新，请刷新确认。',id:'ID',pages:'页数',error:'错误',unknown:'—',QUEUED:'排队中',RIPPING:'渲染中',COMPLETED:'光栅已导出',CANCELLED:'已取消',FAILED:'失败',HELD:'已暂停',RECEIVED:'已接收',VALIDATING:'验证中',PREFLIGHT:'预检',SPOOLING:'保存中',COLOR_PROCESSING:'色彩转换中',HALFTONING:'加网中',PRINTING:'打印中'}
};
let language = navigator.language.startsWith('ja') ? 'ja' : navigator.language.startsWith('zh') ? 'zh-CN' : 'en';
let session = null, jobs = [], selected = null, offset = 0, timer, refreshing = false, operation = false;
let pages = [], previewURL = null, selectionVersion = 0, previewVersion = 0;
let noticeKey = '', noticeDetail = '';
const t = key => texts[language][key] || key;
const engineName = job => ({mupdf:'MuPDF',ghostscript:'Ghostscript'})[job.selected_engine||job.manifest.engine]||t('automatic');
const terminal = state => ['COMPLETED','CANCELLED','FAILED'].includes(state);
class Stale extends Error {}
function notify(key, detail='') { $('notice').className=['submitted','cancelled','held','released'].includes(key)?'success':'error'; noticeKey=key; noticeDetail=detail; $('notice').textContent=t(key); if(detail){const code=document.createElement('small');code.textContent=detail;$('notice').append(code);} }
function translate() {
 document.documentElement.lang=language; $('language').value=language;
 document.querySelectorAll('[data-i18n]').forEach(el=>{el.textContent=t(el.dataset.i18n);});
 document.querySelectorAll('[data-title]').forEach(el=>{el.title=t(el.dataset.title);el.setAttribute('aria-label',el.title);});
 $('connection').querySelector('span').textContent=t(session ? 'disconnect':'connect');
 $('preview-image').alt=t('preview');
 if(noticeKey) notify(noticeKey,noticeDetail);
 renderJobs(); renderDetails();
}
function resetPreview(message='selectJob') {
 previewVersion++; if(previewURL) URL.revokeObjectURL(previewURL); previewURL=null;
 $('preview-image').hidden=true; $('preview-image').removeAttribute('src');
 $('preview-message').hidden=false; $('preview-message').textContent=t(message);
}
function disconnect() {
 session?.abort.abort(); session=null; clearTimeout(timer); operation=false; refreshing=false;
 jobs=[];selected=null;pages=[];offset=0;selectionVersion++;resetPreview();
 $('page').replaceChildren();$('page').disabled=true;$('token').value='';
 notify('');translate();controls();
}
function controls() {
 $('submit').disabled=!session||operation;
 $('refresh').disabled=!session||refreshing;
 $('previous').disabled=!session||refreshing||offset===0;
 $('next').disabled=!session||refreshing||jobs.length<50;
 $('hold').disabled=!selected||operation||!['QUEUED','HELD','VALIDATING','PREFLIGHT'].includes(selected.state);
 $('cancel').disabled=!selected||operation||terminal(selected.state);
 $('hold').querySelector('span').textContent=t(selected?.state==='HELD'?'release':'hold');
}
async function request(path, options={}, s=session) {
 if(!s) throw new Stale();
 let response;
 try { response=await fetch(path,{...options,headers:{...options.headers,Authorization:`Bearer ${s.token}`,'Accept-Language':language},signal:s.abort.signal,credentials:'omit',cache:'no-store',redirect:'error'}); }
 catch(error) { if(s!==session||s.abort.signal.aborted) throw new Stale(); throw error; }
 if(s!==session) throw new Stale();
 if(!response.ok) {
  const error=new Error(); error.status=response.status;
  try { const body=await response.json();error.code=body.code||body.error?.code||''; } catch {}
  if(response.status===401){disconnect();notify('expired');}
  throw error;
 }
 return response;
}
async function json(path, options={}, s=session){ return (await request(path,options,s)).json(); }
function report(error, submission=false) {
 if(error instanceof Stale) return;
 if(error.status===401)return;
 notify(error.status===403?'denied':error.status===409?'conflict':submission&&!error.status?'ambiguous':error.status?'rejected':'network', error.status ? `${error.status}${error.code?` · ${error.code}`:''}` : '');
}
function renderJobs() {
 $('jobs').replaceChildren();
 for(const job of jobs){
  const row=document.createElement('tr');if(selected?.id===job.id)row.className='selected';
  const name=document.createElement('td'), button=document.createElement('button');button.className='job-select';button.textContent=job.manifest.name;button.title=job.manifest.name;button.setAttribute('aria-pressed',String(selected?.id===job.id));button.onclick=()=>selectJob(job);name.append(button);row.append(name);
  const state=document.createElement('td'), badge=document.createElement('span');badge.className=`badge ${job.state}`;badge.textContent=t(job.state);state.append(badge);row.append(state);
  for(const value of [engineName(job),new Date(job.updated_at*1000).toLocaleString(language)]){const td=document.createElement('td');td.textContent=value;row.append(td);}
  $('jobs').append(row);
 }
 $('empty').hidden=jobs.length>0;$('empty').textContent=t(session?'empty':'disconnected');
 $('count').textContent=jobs.length?`${offset+1}–${offset+jobs.length}`:'';
 $('range').textContent=jobs.length?`${offset+1}–${offset+jobs.length}`:'';
 controls();
}
function renderDetails() {
 $('selected-name').textContent=selected?.manifest.name||t('selectJob');$('metadata').replaceChildren();
 if(selected){
  const values=[['id',selected.id],['state',t(selected.state)],['engine',engineName(selected)],['resolution',selected.manifest.dpi],['color',selected.manifest.color_mode.toUpperCase()]];
  if(pages.length)values.push(['pages',pages.length]);
  if(selected.error_code)values.push(['error',selected.error_code]);
  for(const [label,value]of values){const dt=document.createElement('dt'),dd=document.createElement('dd');dt.textContent=t(label);dd.textContent=String(value);$('metadata').append(dt,dd);}
 }
 controls();
}
async function refresh() {
 const s=session;if(!s||refreshing)return;refreshing=true;controls();
 try {
  const result=await json(`/api/v1/jobs?limit=50&offset=${offset}`,{},s);if(s!==session)return;jobs=result;
  if(selected){
   const next=jobs.find(j=>j.id===selected.id)||await json(`/api/v1/jobs/${selected.id}`,{},s);if(s!==session)return;
   if(next){const changed=next.revision!==selected.revision;selected=next;renderDetails();if(changed&&next.state==='COMPLETED')await loadPages(next);}
  }
  renderJobs();
 }catch(error){report(error);}finally{if(s===session){refreshing=false;controls();clearTimeout(timer);timer=setTimeout(()=>{if(!document.hidden)refresh();else schedule();},3000);}}
}
function schedule(){clearTimeout(timer);if(session)timer=setTimeout(()=>{if(document.hidden)schedule();else refresh();},3000);}
async function selectJob(job){if(selected?.id!==job.id)notify('');selected=job;pages=[];const version=++selectionVersion;resetPreview('loading');$('page').replaceChildren();$('page').disabled=true;renderJobs();renderDetails();await loadPages(job,version);}
async function loadPages(job,version=selectionVersion){
 if(job.state!=='COMPLETED'){resetPreview('noPreview');return;}
 const s=session;
 try{
  const result=await json(`/api/v1/jobs/${encodeURIComponent(job.id)}/artifacts`,{},s);
  if(s!==session||version!==selectionVersion||selected?.id!==job.id)return;
  pages=result;$('page').replaceChildren();
  for(const page of pages){const option=document.createElement('option');option.value=page.name;option.textContent=String(page.page);$('page').append(option);}
  $('page').disabled=!pages.length;renderDetails();if(pages.length)await showPage();else resetPreview('noPreview');
 }catch(error){if(version===selectionVersion){resetPreview('noPreview');report(error);}}
}
async function showPage(){
 if(!selected||!$('page').value)return;
 const s=session,id=selected.id,name=$('page').value;resetPreview('loading');const version=previewVersion;
 try{
  const response=await request(`/api/v1/jobs/${encodeURIComponent(id)}/artifacts/${encodeURIComponent(name)}/preview`,{},s);
  const blob=await response.blob();if(s!==session||version!==previewVersion)return;
  previewURL=URL.createObjectURL(blob);$('preview-image').src=previewURL;$('preview-image').hidden=false;$('preview-message').hidden=true;
 }catch(error){if(s===session&&version===previewVersion){resetPreview('unsupported');if(error.status!==422)report(error);}}
}
async function control(action){
 if(!selected||operation)return;const s=session,job=selected;operation=true;controls();
 try{const updated=await json(`/api/v1/jobs/${job.id}/${action}`,{method:'POST',headers:{'x-jrip-expected-revision':String(job.revision)}},s);if(s!==session)return;const index=jobs.findIndex(j=>j.id===job.id);if(index>=0)jobs[index]=updated;if(selected?.id===job.id)selected=updated;notify(action==='cancel'?'cancelled':action==='hold'?'held':'released');renderJobs();renderDetails();}
 catch(error){report(error);if(error.status===409)await refresh();}
 finally{if(s===session){operation=false;controls();}}
}
$('upload').addEventListener('submit',async event=>{
 event.preventDefault();if(!session||operation)return;
 const s=session,file=$('file').files[0];if(!file||file.size===0||file.size>32*1024*1024){notify('fileInvalid');return;}
 operation=true;controls();
 try{
  const bytes=new Uint8Array(await file.slice(0,100).arrayBuffer());const signature=new TextDecoder().decode(bytes);
  const format=signature.startsWith('%PDF-')?'application/pdf':signature.startsWith('%!')?(signature.includes('EPSF')?'application/eps':'application/postscript'):null;
  if(!format){notify('fileInvalid');return;}
  const manifest={name:file.name,format,dpi:Number($('dpi').value),color_mode:$('color').value,engine:$('engine').value,priority:Number($('priority').value)};
  const body=new FormData();body.append('manifest',JSON.stringify(manifest));body.append('document',file);
  const job=await json('/api/v1/jobs',{method:'POST',body},s);if(s!==session)return;
  offset=0;await refresh();await selectJob(job);notify('submitted');
 }catch(error){report(error,true);}finally{if(s===session){operation=false;controls();}}
});
$('connection').onclick=()=>{if(session){disconnect();return;}$('login').showModal();$('token').focus();};
$('close-login').onclick=()=>{$('token').value='';$('login').close();};
$('login').addEventListener('cancel',()=>{$('token').value='';});
$('login-form').onsubmit=async event=>{
 event.preventDefault();if(session)return;
 const token=$('token').value.trim();$('token').value='';if(!token)return;
 session={token,abort:new AbortController()};const s=session;
 const button=$('login-form').querySelector('[type=submit]');button.disabled=true;
 try{jobs=await json('/api/v1/jobs?limit=50&offset=0',{},s);if(s!==session)return;$('login').close();notify('');translate();controls();schedule();}
 catch(error){if(s===session)disconnect();report(error);}
 finally{button.disabled=false;}
};
$('language').onchange=()=>{language=$('language').value;translate();if(!previewURL)$('preview-message').textContent=t(selected?'noPreview':'selectJob');};
$('refresh').onclick=refresh;$('page').onchange=showPage;
$('previous').onclick=()=>{offset=Math.max(0,offset-50);refresh();};$('next').onclick=()=>{offset+=50;refresh();};
$('hold').onclick=()=>control(selected?.state==='HELD'?'release':'hold');$('cancel').onclick=()=>control('cancel');
$('file').onchange=()=>{$('file-name').textContent=$('file').files[0]?.name||'PDF · PS · EPS';};
for(const event of ['dragenter','dragover'])$('drop').addEventListener(event,e=>{e.preventDefault();$('drop').classList.add('drag');});
for(const event of ['dragleave','drop'])$('drop').addEventListener(event,e=>{e.preventDefault();$('drop').classList.remove('drag');});
$('drop').addEventListener('drop',event=>{if(event.dataTransfer.files.length===1){$('file').files=event.dataTransfer.files;$('file').dispatchEvent(new Event('change'));}});
document.addEventListener('visibilitychange',()=>{if(!document.hidden)refresh();});
window.addEventListener('pagehide',()=>disconnect());
translate();controls();
