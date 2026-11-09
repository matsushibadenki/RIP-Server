'use strict';
// This renderer owns no session or authorization state; all device strings are text.
window.JripPrinterStatus=(()=>{
 const labels={
  en:{printerStatus:'Printer status',supplies:'Supplies',refreshPrinter:'Refresh printer',printerStatusHelp:'Device-reported information, not print approval or a page estimate. PWG quantities and CUPS levels are separate sources. Refreshing invalidates the output review.',printerQueueHelp:'Device-reported pending, held, processing and stopped jobs; not the RIP queue or an ETA.',printerJobs:'Device jobs',printerAccepting:'Accepting jobs',printerNotAccepting:'Not accepting jobs',printerSnapshotExpired:'Expired snapshot',printerSnapshotStored:'Saved report',printerSnapshotCurrent:'Snapshot',printerQueryFailed:'Query unavailable · last report',printerNoNotices:'No notices reported',printerNoticesUnknown:'Notices not reported',printerInconsistent:'Inconsistent device report',PS_3:'Idle',PS_4:'Processing',PS_5:'Stopped',S_UNKNOWN:'Unknown',S_UNREPORTED:'Not reported',S_UNAVAILABLE:'Unavailable',S_OTHER:'Other / unspecified',S_NOT_AT_CAPACITY:'Below capacity · amount unknown',S_SOME_AVAILABLE:'Some available · amount unknown',S_UNSUPPORTED:'Not supported',S_NO_VALUE:'No value',S_INVALID_RESPONSE:'Invalid supply report',currentSupply:'Current supply',remainingSpace:'Remaining space',reportedLevel:'Reported level',unitUnknown:'Unit not reported',supply:'Supply',C_mediaEmpty:'Paper empty',C_mediaLow:'Paper low',C_supplyLow:'Supply low',C_wasteAlmostFull:'Waste nearly full',C_wasteFull:'Waste full',C_paused:'Paused',C_stopping:'Stopping',C_connecting:'Connecting',C_timedOut:'Device timed out',C_trayMissing:'Input tray missing',C_outputFull:'Output area full',C_outputAlmostFull:'Output area nearly full',C_outputMissing:'Output tray missing',C_interlock:'Interlock open',C_other:'Device notice',C_unknown:'Unknown notice'},
  ja:{printerStatus:'プリンター状態',supplies:'消耗品',refreshPrinter:'プリンターを更新',printerStatusHelp:'機器の報告値です。印刷承認や印刷可能枚数ではありません。PWGの数量とCUPSの残量は別の情報源です。更新すると出力条件の確認を解除します。',printerQueueHelp:'機器が報告した待機・保留・印刷中・停止中の件数です。RIP側の件数や待ち時間ではありません。',printerJobs:'機器ジョブ数',printerAccepting:'受付可能',printerNotAccepting:'受付停止',printerSnapshotExpired:'有効期限切れ',printerSnapshotStored:'保存済みの報告',printerSnapshotCurrent:'照会時点',printerQueryFailed:'照会不可・前回の報告',printerNoNotices:'通知の報告なし',printerNoticesUnknown:'通知情報なし',printerInconsistent:'機器報告に不整合',PS_3:'待機中',PS_4:'処理中',PS_5:'停止中',S_UNKNOWN:'不明',S_UNREPORTED:'未報告',S_UNAVAILABLE:'取得不可',S_OTHER:'その他・未指定',S_NOT_AT_CAPACITY:'容量未満・数量不明',S_SOME_AVAILABLE:'一部あり・数量不明',S_UNSUPPORTED:'未対応',S_NO_VALUE:'値なし',S_INVALID_RESPONSE:'消耗品の報告が不正',currentSupply:'現在量',remainingSpace:'残り空き容量',reportedLevel:'報告値',unitUnknown:'単位未報告',supply:'消耗品',C_mediaEmpty:'用紙切れ',C_mediaLow:'用紙残量少',C_supplyLow:'消耗品残量少',C_wasteAlmostFull:'廃棄容器が満杯間近',C_wasteFull:'廃棄容器が満杯',C_paused:'一時停止',C_stopping:'停止移行中',C_connecting:'接続中',C_timedOut:'機器の応答待ち',C_trayMissing:'給紙トレイ未装着',C_outputFull:'排紙部が満杯',C_outputAlmostFull:'排紙部が満杯間近',C_outputMissing:'排紙トレイ未装着',C_interlock:'インターロック開放',C_other:'機器の通知',C_unknown:'未対応の通知'},
  'zh-CN':{printerStatus:'打印机状态',supplies:'耗材',refreshPrinter:'刷新打印机',printerStatusHelp:'设备报告值，不是打印批准或可打印页数。PWG数量与CUPS余量是独立来源。刷新将清除输出条件确认。',printerQueueHelp:'设备报告的等待、保留、打印与停止作业数，不是RIP队列或预计等待时间。',printerJobs:'设备作业数',printerAccepting:'接受作业',printerNotAccepting:'停止接收',printerSnapshotExpired:'快照已过期',printerSnapshotStored:'已保存报告',printerSnapshotCurrent:'查询时点',printerQueryFailed:'无法查询・上次报告',printerNoNotices:'未报告通知',printerNoticesUnknown:'通知信息未提供',printerInconsistent:'设备报告不一致',PS_3:'空闲',PS_4:'处理中',PS_5:'已停止',S_UNKNOWN:'未知',S_UNREPORTED:'未报告',S_UNAVAILABLE:'无法获取',S_OTHER:'其他／未指定',S_NOT_AT_CAPACITY:'未满・数量未知',S_SOME_AVAILABLE:'有部分可用・数量未知',S_UNSUPPORTED:'不支持',S_NO_VALUE:'无值',S_INVALID_RESPONSE:'耗材报告无效',currentSupply:'当前数量',remainingSpace:'剩余空间',reportedLevel:'报告值',unitUnknown:'单位未报告',supply:'耗材',C_mediaEmpty:'纸张用尽',C_mediaLow:'纸张余量低',C_supplyLow:'耗材余量低',C_wasteAlmostFull:'废料容器将满',C_wasteFull:'废料容器已满',C_paused:'已暂停',C_stopping:'正在停止',C_connecting:'正在连接',C_timedOut:'设备响应超时',C_trayMissing:'未安装进纸盒',C_outputFull:'出纸区域已满',C_outputAlmostFull:'出纸区域将满',C_outputMissing:'未安装出纸盘',C_interlock:'联锁打开',C_other:'设备通知',C_unknown:'未知通知'}
 };
 const supplyNames={
  en:{K_toner:'Toner',K_ink:'Ink',K_wasteToner:'Waste toner',K_wasteInk:'Waste ink',K_staples:'Staples',K_fuser:'Fuser',K_transfer:'Transfer unit',K_other:'Supply',colorBlack:'Black',colorCyan:'Cyan',colorMagenta:'Magenta',colorYellow:'Yellow',colorMulti:'Multicolor',unitItems:'items',unitSheets:'sheets',unitImpressions:'impressions',unitHours:'hours'},
  ja:{K_toner:'トナー',K_ink:'インク',K_wasteToner:'廃トナー',K_wasteInk:'廃インク',K_staples:'ステープル',K_fuser:'定着器',K_transfer:'転写ユニット',K_other:'消耗品',colorBlack:'黒',colorCyan:'シアン',colorMagenta:'マゼンタ',colorYellow:'黄',colorMulti:'複数色',unitItems:'個',unitSheets:'枚',unitImpressions:'印刷面',unitHours:'時間'},
  'zh-CN':{K_toner:'碳粉',K_ink:'墨水',K_wasteToner:'废碳粉',K_wasteInk:'废墨',K_staples:'订书钉',K_fuser:'定影器',K_transfer:'转印组件',K_other:'耗材',colorBlack:'黑',colorCyan:'青',colorMagenta:'品红',colorYellow:'黄',colorMulti:'多色',unitItems:'个',unitSheets:'张',unitImpressions:'印面',unitHours:'小时'}
 };
 for(const lang of Object.keys(labels))Object.assign(labels[lang],supplyNames[lang]);
 function fallbackName(item){
  const kind=String(item.kind||'').toLowerCase().replaceAll('-','');
  const key={toner:'K_toner',tonercartridge:'K_toner',ink:'K_ink',inkcartridge:'K_ink',wastetoner:'K_wasteToner',wasteink:'K_wasteInk',staples:'K_staples',fuser:'K_fuser',transferunit:'K_transfer',other:'K_other'}[kind];
  const colorKey={black:'colorBlack',cyan:'colorCyan',magenta:'colorMagenta',yellow:'colorYellow','multi-color':'colorMulti'}[item.colorantName];
  const color=item.colorantName==='no-color'?'':colorKey?t(colorKey):item.colorantName||item.color;
  return [color,key?t(key):item.kind].filter(Boolean).join(' · ');
 }
 function unitLabel(unit){const key={items:'unitItems',sheets:'unitSheets',impressions:'unitImpressions',hours:'unitHours'}[unit];return key?t(key):unit||'?';}
 for(const lang of Object.keys(labels))Object.assign(texts[lang],labels[lang]);
 const reasonKeys={'media-empty':'C_mediaEmpty','media-needed':'R_mediaNeeded','media-low':'C_mediaLow','media-jam':'R_mediaJam','toner-low':'C_supplyLow','toner-empty':'R_tonerEmpty','marker-supply-low':'C_supplyLow','marker-supply-empty':'R_supplyEmpty','marker-waste-almost-full':'C_wasteAlmostFull','marker-waste-full':'C_wasteFull','cover-open':'R_coverOpen','door-open':'R_doorOpen','interlock-open':'C_interlock','input-tray-missing':'C_trayMissing','output-area-full':'C_outputFull','output-area-almost-full':'C_outputAlmostFull','output-tray-missing':'C_outputMissing','paused':'C_paused','moving-to-paused':'C_stopping','stopping':'C_stopping','stopped-partly':'PS_5','connecting-to-device':'C_connecting','timed-out':'C_timedOut','other':'C_other'};
 function element(tag,cls,text){const node=document.createElement(tag);if(cls)node.className=cls;if(text!==undefined)node.textContent=String(text);return node;}
 function info(message){const button=element('button','icon printer-info','ⓘ');button.type='button';button.title=message;button.setAttribute('aria-label',message);return button;}
 function numeric(value){return Number.isInteger(value)&&value>=0;}
 function status(key){return Object.hasOwn(labels[language],`S_${key}`)?t(`S_${key}`):t('S_UNKNOWN');}
 function sourceList(id,source,report,items,pwg,prefix){
  const root=document.getElementById(prefix+id);root.replaceChildren();
  if(report?.status!=='REPORTED'||!Array.isArray(items)||!items.length){const row=element('div','supply-empty');row.append(element('small','source-tag',source),element('span','muted',status(report?.status==='REPORTED'?'UNREPORTED':report?.status==='INVALID_RESPONSE'?'INVALID_RESPONSE':(pwg?report?.attributeState:report?.levelsAttributeState)||'UNREPORTED')));if(report?.status==='INVALID_RESPONSE')row.append(info(`${report.messages?.[language]||t('S_INVALID_RESPONSE')} · ${report.reasonCode||''}`));root.append(row);return;}
  for(const item of items){
   const name=(pwg?item.description?.text:item.name)||fallbackName(item);
   const meaning=pwg?({CURRENT_SUPPLY:'currentSupply',REMAINING_SPACE:'remainingSpace'}[item.levelMeaning]||'reportedLevel'):'reportedLevel';
   const percent=numeric(item.levelPercent)&&item.levelPercent<=100?item.levelPercent:null;
   const quantity=pwg&&numeric(item.level)?`${item.level.toLocaleString(language)} ${unitLabel(item.unit)}`:null;
   const text=percent!==null?`${percent.toLocaleString(language)} %`:(quantity||status(item.levelState));
   const card=element('div','supply-card'),head=element('div','supply-head'),meta=element('div','supply-meta');
   head.append(element('strong','supply-name',name||`${t('supply')} ${item.index||item.position||''}`),element('span','supply-value',text));
   meta.append(element('small','source-tag',source),element('small','muted',t(meaning)));
   const detail=[name,t(meaning),pwg&&!item.unit?t('unitUnknown'):item.unit,item.kind,pwg?item.supplyClass:null].filter(Boolean).join(' · ');
   card.title=detail;card.append(head,meta);
   if(percent!==null){const meter=element('meter','supply-meter');meter.min=0;meter.max=100;meter.value=percent;meter.setAttribute('aria-label',`${name||t('supply')} · ${t(meaning)}`);card.append(meter);}
   root.append(card);
  }
 }
 function render({snapshot,error,loading,selected,prefix=''}){
  const $=id=>document.getElementById(prefix+id);
  const root=$('printer-live');root.hidden=!selected;if(!selected){for(const id of ['printer-facts','printer-conditions','printer-pwg-supplies','printer-cups-supplies'])$(id).replaceChildren();$('printer-query-state').textContent='';$('printer-snapshot-content').hidden=true;return;}
  const expired=Boolean(snapshot&&(!Number.isFinite(snapshot.expiresAt)||snapshot.expiresAt*1000<=Date.now()));
  const stale=Boolean(error||expired);
  root.dataset.source=snapshot?(snapshot.stored?'stored':'live'):'none';root.dataset.stale=String(stale);root.setAttribute('aria-busy',String(Boolean(loading)));
  const date=Number.isFinite(snapshot?.observedAt)?new Date(snapshot.observedAt*1000).toLocaleString(language):'';
  const queryLabel=error?t(snapshot?'printerQueryFailed':'Q_UNAVAILABLE'):snapshot?t(stale?'printerSnapshotExpired':'printerSnapshotCurrent'):'';
  const parts=[snapshot?.stored?t('printerSnapshotStored'):null,queryLabel,error&&expired?t('printerSnapshotExpired'):null,date,loading?t('loading'):null].filter(Boolean);
  $('printer-query-state').textContent=parts.join(' · ');
  $('printer-snapshot-content').hidden=!snapshot;if(!snapshot){for(const id of ['printer-facts','printer-conditions','printer-pwg-supplies','printer-cups-supplies'])$(id).replaceChildren();return;}
  const live=snapshot.live||{},facts=$('printer-facts');facts.replaceChildren();
  facts.append(element('span','printer-chip',t(Object.hasOwn(labels[language],`PS_${live.state}`)?`PS_${live.state}`:'S_UNKNOWN')));
  facts.append(element('span','printer-chip',typeof live.acceptingJobs==='boolean'?t(live.acceptingJobs?'printerAccepting':'printerNotAccepting'):t('S_UNKNOWN')));
  const queue=element('span','printer-chip',`${t('printerJobs')}: ${numeric(live.queuedJobCount)?live.queuedJobCount.toLocaleString(language):t('S_UNKNOWN')}`);queue.title=t('printerQueueHelp');facts.append(queue);
  const notices=$('printer-conditions');notices.replaceChildren();
  for(const condition of live.conditions?.conditions||[]){const severity=['error','warning','report'].includes(condition.severity)?condition.severity:'unknown';const row=element('li',`printer-condition condition-${severity}`);const label=condition.known&&reasonKeys[condition.reason]?t(reasonKeys[condition.reason]):`${t('C_unknown')} · ${condition.rawReason}`;row.append(element('span','condition-symbol',severity==='report'?'ⓘ':'!'),element('span','condition-label',label),info([condition.rawReason,condition.severity,condition.recovery?.messages?.[language]].filter(Boolean).join(' · ')));notices.append(row);}
  if(!notices.children.length)notices.append(element('li','muted printer-no-notices',t(live.conditions?.reasonEvidence==='NO_REASONS_REPORTED'?'printerNoNotices':'printerNoticesUnknown')));
  if(live.conditions?.inconsistencies?.length){const row=element('li','printer-condition condition-warning');row.append(element('span','condition-label',t('printerInconsistent')),info(live.conditions.inconsistencies.join(', ')));notices.append(row);}
  sourceList('printer-pwg-supplies','PWG',live.printerSupplies,live.printerSupplies?.supplies,true,prefix);
  sourceList('printer-cups-supplies','CUPS',live.markerSupplies,live.markerSupplies?.markers,false,prefix);
 }
 return {render};
})();
