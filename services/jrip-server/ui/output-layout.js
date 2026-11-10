'use strict';
// Read-only placement: actual TIFF pixels / requested DPI, with no fit-to-paper.
window.JripOutputLayout=(()=>{
 const words={en:{outputLayout:'Paper preview',layoutHelp:'Actual-size raster at the top-left, without scaling or rotation. The outline is the selected paper, not the device printable area. Placement preview only; check output before printing. Duplex back-side transforms are not simulated.',layoutChoose:'Select a printer and paper.',layoutLoading:'Loading paper preview…',layoutUnavailable:'Image preview unavailable',layoutMismatch:'Size mismatch · output cannot be printed with these settings',layoutPosition:'Top-left · actual size',layoutImage:'Image',layoutPaper:'Paper',layoutEmpty:'No rendered pages match this range.'},ja:{outputLayout:'用紙プレビュー',layoutHelp:'ラスターを左上に等倍配置します。拡大縮小・回転は行いません。枠は選択した用紙で、実機の印字可能領域ではありません。配置確認用です。印刷前に「出力を確認」を実行してください。両面の裏面変換は再現しません。',layoutChoose:'プリンターと用紙を選択してください。',layoutLoading:'用紙プレビューを取得中…',layoutUnavailable:'画像プレビューを表示できません',layoutMismatch:'寸法不一致 · この設定では印刷できません',layoutPosition:'左上・等倍',layoutImage:'画像',layoutPaper:'用紙',layoutEmpty:'指定範囲に生成済みページがありません。'},'zh-CN':{outputLayout:'纸张预览',layoutHelp:'光栅按原尺寸置于左上角，不缩放或旋转。边框表示所选纸张，而非设备可打印区域。仅供位置检查，打印前请检查输出。不模拟双面背面变换。',layoutChoose:'请选择打印机和纸张。',layoutLoading:'正在加载纸张预览…',layoutUnavailable:'图像预览不可用',layoutMismatch:'尺寸不符・当前设置无法打印',layoutPosition:'左上角・原尺寸',layoutImage:'图像',layoutPaper:'纸张',layoutEmpty:'指定范围内没有已渲染页面。'}};
 const placementWords={
 en:{placementInvalid:'Enter valid scale and offsets.',placementScale:'Scaling',placementOriginal:'Original · exact size',placementFit:'Fit to paper',placementActual:'Actual size',placementCustom:'Custom scale',placementPercent:'Scale · %',placementRotation:'Rotation · clockwise',placementAlign:'Position',placementX:'Horizontal offset · mm',placementY:'Vertical offset · mm',align00:'Top left',align10:'Top center',align20:'Top right',align01:'Middle left',align11:'Center',align21:'Middle right',align02:'Bottom left',align12:'Bottom center',align22:'Bottom right',placementClipped:'Image extends beyond paper · outside area will be clipped',placementSummary:'Placement',layoutHelp:'Placement on the selected paper. Scaling, clockwise rotation, alignment and offsets are applied to the actual raster output. Outside areas are clipped. Device unprintable margins and duplex back-side transforms are not simulated. Check output and approve before printing.'},
 ja:{placementInvalid:'倍率と移動量を正しく入力してください。',placementScale:'拡大縮小',placementOriginal:'変更なし・寸法一致',placementFit:'用紙に合わせる',placementActual:'等倍',placementCustom:'倍率指定',placementPercent:'倍率・%',placementRotation:'回転・時計回り',placementAlign:'配置位置',placementX:'横位置の調整・mm',placementY:'縦位置の調整・mm',align00:'左上',align10:'上中央',align20:'右上',align01:'左中央',align11:'中央',align21:'右中央',align02:'左下',align12:'下中央',align22:'右下',placementClipped:'用紙の外にはみ出しています・枠外は切り取られます',placementSummary:'配置',layoutHelp:'選択した用紙への配置です。拡大縮小・時計回りの回転・配置位置・移動量を実際のラスター出力にも適用します。用紙外は切り取ります。実機の非印字領域・両面裏面変換の再現ではありません。出力確認・承認後に印刷してください。'},
 'zh-CN':{placementInvalid:'请输入有效比例及偏移量。',placementScale:'缩放',placementOriginal:'不变・尺寸须相符',placementFit:'适合纸张',placementActual:'原尺寸',placementCustom:'指定比例',placementPercent:'比例・%',placementRotation:'顺时针旋转',placementAlign:'位置',placementX:'水平偏移・mm',placementY:'垂直偏移・mm',align00:'左上',align10:'上中',align20:'右上',align01:'左中',align11:'居中',align21:'右中',align02:'左下',align12:'下中',align22:'右下',placementClipped:'图像超出纸张・框外区域将被裁切',placementSummary:'布局',layoutHelp:'按所选纸张显示布局。缩放、顺时针旋转、对齐与偏移同时应用于实际光栅输出，裁切纸张外区域。不模拟设备不可打印边距及双面背面变换。打印前须检查输出并审批。'}};
 for(const lang of Object.keys(placementWords))Object.assign(words[lang],placementWords[lang]);
 function placement(){
  const mode=$('placement-mode').value;if(mode==='original')return null;
  const percent=mode==='custom'?Number($('placement-percent').value):100,rotation=Number($('placement-rotation').value),[alignX,alignY]=$('placement-align').value.split(',').map(Number),x=Number($('placement-x').value),y=Number($('placement-y').value);
  if(![percent,rotation,alignX,alignY,x,y].every(Number.isFinite)||percent<1||percent>400||Math.abs(x)>1000||Math.abs(y)>1000||!$('placement-x').value||!$('placement-y').value||(mode==='custom'&&!$('placement-percent').value))return false;
  return {scaleMode:mode,scaleBasisPoints:Math.round(percent*100),rotation,alignX,alignY,offsetXMicrons:Math.round(x*1000),offsetYMicrons:Math.round(y*1000)};
 }
 function resetControls(){for(const [id,value]of [['placement-mode','original'],['placement-percent','100'],['placement-rotation','0'],['placement-align','0,0'],['placement-x','0'],['placement-y','0']])$(id).value=value;}
 function summary(p){return `${t(p.scaleMode==='fit'?'placementFit':p.scaleMode==='actual'?'placementActual':'placementCustom')} ${p.scaleMode==='custom'?p.scaleBasisPoints/100+'% ':''}· ${p.rotation}° · ${t('align'+p.alignX+p.alignY)} · ${p.offsetXMicrons/1000}, ${p.offsetYMicrons/1000} mm`;}
 for(const lang of Object.keys(words))Object.assign(texts[lang],words[lang]);
 let key='',version=0,abort=null,url=null,input=null,available=[],facts=null,message='layoutChoose',imageMissing=false,choice='';
 function clear(){$('output-layout-dimensions').textContent='';version++;abort?.abort();abort=null;if(url)URL.revokeObjectURL(url);url=null;facts=null;imageMissing=false;$('output-layout-image').removeAttribute('src');$('output-layout-image').hidden=true;$('output-layout-stage').hidden=true;}
 function reset(){clear();key='';input=null;available=[];choice='';message='layoutChoose';$('output-layout-page').replaceChildren();render();}
 function geometry(raster,media,dpi,p=null){
  const values=[raster?.width,raster?.height,media?.widthMicrons,media?.heightMicrons,dpi];if(!values.every(n=>Number.isFinite(n)&&n>0))return null;
  const w=raster.width/dpi*25.4,h=raster.height/dpi*25.4,pw=media.widthMicrons/1000,ph=media.heightMicrons/1000;
  const resolution=raster.resolution;
  const wrongDpi=resolution&&(!['x','y'].every(axis=>{const r=resolution[axis];return Array.isArray(r)&&r.length===2&&r[1]>0&&Math.abs(r[0]/r[1]-dpi)<=.01;}));
  if(p===false)return null;
  if(p){
   const paperW=Math.round(media.widthMicrons*dpi/25400),paperH=Math.round(media.heightMicrons*dpi/25400),turn=p.rotation%180!==0,rw=turn?raster.height:raster.width,rh=turn?raster.width:raster.height;
   const scale=p.scaleMode==='fit'?Math.min(paperW/rw,paperH/rh):p.scaleMode==='custom'?p.scaleBasisPoints/10000:1;
   const iw=Math.max(1,Math.round(rw*scale)),ih=Math.max(1,Math.round(rh*scale)),x=Math.round((paperW-iw)*p.alignX/2)+Math.round(p.offsetXMicrons*dpi/25400),y=Math.round((paperH-ih)*p.alignY/2)+Math.round(p.offsetYMicrons*dpi/25400);
   return {w:iw/dpi*25.4,h:ih/dpi*25.4,pw,ph,widthPercent:iw/paperW*100,heightPercent:ih/paperH*100,leftPercent:x/paperW*100,topPercent:y/paperH*100,rotation:p.rotation,mismatch:Boolean(wrongDpi),clipped:x<0||y<0||x+iw>paperW||y+ih>paperH};
  }
  return {leftPercent:0,topPercent:0,rotation:0,w,h,pw,ph,widthPercent:w/pw*100,heightPercent:h/ph*100,mismatch:Boolean(wrongDpi)||Math.abs(raster.width-Math.round(pw/25.4*dpi))>1||Math.abs(raster.height-Math.round(ph/25.4*dpi))>1};
 }
 function render(){
  const status=$('output-layout-status');status.textContent=message?t(message):'';status.className='muted';
  const page=$('output-layout-page');page.setAttribute('aria-label',t('sourcePage'));page.disabled=!available.length;page.hidden=!available.length;
  const g=facts&&input&&geometry(facts,input.media,input.job.manifest.dpi,input.placement);if(!g){if(input?.placement===false){$('output-layout-stage').hidden=true;$('output-layout-dimensions').textContent='';status.textContent=t('placementInvalid');}return;}
  const paper=$('output-layout-paper'),image=$('output-layout-image'),outline=$('output-layout-bounds'),position=$('output-layout-position');
  paper.style.aspectRatio=`${g.pw} / ${g.ph}`;paper.style.width=`min(100%, ${Math.min(360,300*g.pw/g.ph)}px)`;
  for(const e of [position,outline]){e.style.left=`${g.leftPercent}%`;e.style.top=`${g.topPercent}%`;e.style.width=`${g.widthPercent}%`;e.style.height=`${g.heightPercent}%`;}
  image.style.width=`${g.rotation%180?g.h/g.w*100:100}%`;image.style.height=`${g.rotation%180?g.w/g.h*100:100}%`;image.style.transform=`translate(-50%,-50%) rotate(${g.rotation}deg)`;
  paper.classList.toggle('layout-mismatch',g.mismatch||g.clipped);$('output-layout-stage').hidden=false;
  const fmt=n=>n.toLocaleString(language,{maximumFractionDigits:2});
  $('output-layout-dimensions').textContent=`${t('layoutPaper')} ${fmt(g.pw)} × ${fmt(g.ph)} mm · ${t('layoutImage')} ${fmt(g.w)} × ${fmt(g.h)} mm · ${input.placement?summary(input.placement):t('layoutPosition')}`;
  status.textContent=g.mismatch?t('layoutMismatch'):g.clipped?t('placementClipped'):imageMissing?t('layoutUnavailable'):'';status.className=g.mismatch||g.clipped?'layout-warning':'muted';
  image.alt=`${t('outputLayout')} · ${t('sourcePage')} ${available.find(a=>a.name===choice)?.sourcePage||available.find(a=>a.name===choice)?.page||''}`;
 }
 async function get(path,s,signal){
  for(let n=0;;n++){try{return await request(path,{signal},s);}catch(e){if(e.status!==503||e.code!=='artifact_budget_exhausted'||n>=3||signal.aborted)throw e;await new Promise(r=>setTimeout(r,250*2**n));}}
 }
 async function draw(){
  clear();message='layoutLoading';render();const v=version,s=input?.s;if(!s)return;abort=new AbortController();const signal=abort.signal;const valid=()=>v===version&&s===session&&!signal.aborted;
  const a=available.find(a=>a.name===choice);if(!a)return;
  const base=input.color?`/api/v1/color/attempts/${encodeURIComponent(input.color)}/artifacts`:`/api/v1/jobs/${input.job.id}/artifacts`;
  const path=`${base}/${encodeURIComponent(a.name)}`;
  try{
   const d=await (await get(`${path}/descriptor`,s,signal)).json();if(!valid())return;
   if(!d.integrityVerified||d.sha256!==a.sha256||!geometry(d.raster,input.media,input.job.manifest.dpi))throw new Error('invalid layout facts');
   facts=d.raster;message='';render();
   try{const response=await get(`${path}/preview`,s,signal),blob=await response.blob();if(!valid())return;url=URL.createObjectURL(blob);$('output-layout-image').src=url;$('output-layout-image').hidden=false;}
   catch(e){if(!valid())return;imageMissing=true;}render();
  }catch(e){if(valid()){message='layoutUnavailable';render();}}
 }
 async function update(next){
  if(!next||!$('delivery-panel').open){if(key||input)reset();return;}
  const ranges=parsePageRanges(next.ranges),newKey=JSON.stringify([next.job.id,next.job.revision,next.color,next.media,ranges,next.pages.map(a=>[a.name,a.sha256])]);
  if(next.s===input?.s&&newKey===key){input=next;render();return;}
  clear();key=newKey;input=next;available=[];$('output-layout-page').replaceChildren();message=next.media?'layoutLoading':'layoutChoose';render();if(!next.media)return;
  const v=version;let list=next.pages;
  try{
   if(next.color){abort=new AbortController();const result=await json(`/api/v1/color/attempts/${encodeURIComponent(next.color)}`,{signal:abort.signal},next.s);if(v!==version||next.s!==session)return;list=result.result?.artifacts||[];}
   if(v!==version||next.s!==session)return;
   available=list.filter(a=>ranges&&(!ranges.length||ranges.some(r=>(a.sourcePage||a.page)>=r.first&&(a.sourcePage||a.page)<=r.last)));
   if(!available.length){message='layoutEmpty';render();return;}
   if(!available.some(a=>a.name===choice))choice=available[0].name;
   const page=$('output-layout-page');for(const a of available)page.append(new Option(String(a.sourcePage||a.page),a.name));page.value=choice;await draw();
  }catch(e){if(v===version&&next.s===session){message='layoutUnavailable';render();}}
 }
 $('output-layout-page').onchange=()=>{choice=$('output-layout-page').value;draw();};
 return {update,reset,render,geometry,placement,resetControls,summary};
})();
translate();
