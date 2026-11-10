 'use strict';
window.JripTabs=(()=>{
 const words={en:{outputTabEmpty:'Select a completed job in Jobs.',workspaceTabs:'Workspace'},ja:{outputTabEmpty:'ジョブタブで完了したジョブを選択してください。',workspaceTabs:'操作画面'},'zh-CN':{outputTabEmpty:'请在任务选项卡中选择已完成任务。',workspaceTabs:'工作区'}};
 for(const lang of Object.keys(words))Object.assign(texts[lang],words[lang]);
 const names=['jobs','output','printers','fonts'];let active='jobs';
 function sync(){
  document.querySelector('main').dataset.tab=active;document.querySelector('[role="tablist"]').setAttribute('aria-label',t('workspaceTabs'));
  for(const name of names){const tab=$('tab-'+name);tab.setAttribute('aria-selected',String(name===active));tab.tabIndex=name===active?0:-1;$('view-'+name).hidden=name!==active;}
  $('job-upload-panel').hidden=active!=='jobs';
  const output=$('delivery-panel'),printers=$('printer-dashboard');
  output.open=active==='output'&&!output.hidden;printers.open=active==='printers'&&!printers.hidden;
  window.JripFonts?.sync(active==='fonts');
  $('output-tab-empty').hidden=!output.hidden;$('printers-tab-empty').hidden=!printers.hidden;
 }
 function select(name,focus=false){active=name;sync();if(focus)$('tab-'+name).focus();}
 for(const [index,name]of names.entries()){
  $('tab-'+name).onclick=()=>select(name);
  $('tab-'+name).onkeydown=event=>{let target;if(event.key==='ArrowRight')target=(index+1)%names.length;else if(event.key==='ArrowLeft')target=(index+names.length-1)%names.length;else if(event.key==='Home')target=0;else if(event.key==='End')target=names.length-1;else return;event.preventDefault();select(names[target],true);};
 }
 return {sync,reset:()=>select('jobs')};
})();
translate();
