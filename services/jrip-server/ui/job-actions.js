'use strict';
window.JripJobActions=(()=>{
 const words={en:{jobActions:'Actions',revealRaster:'Show raster in Finder',deleteJob:'Delete job',deleteJobHelp:'Remove this job from the list. Source files and raster data are retained for separate retention cleanup.',deleteApproval:'Deletion approval ID',deleted:'Job deleted',revealed:'Raster selected in Finder',finderUnavailable:'Finder access is available only on the local Mac.',deleteApprovalNeeded:'Enter a deletion approval ID.'},ja:{jobActions:'操作',revealRaster:'ラスターをFinderに表示',deleteJob:'ジョブを削除',deleteJobHelp:'ジョブを一覧から削除します。原稿とラスターデータは保持され、別途保存期限の管理で削除します。',deleteApproval:'削除の承認ID',deleted:'ジョブを削除しました',revealed:'Finderでラスターを選択しました',finderUnavailable:'FinderへのアクセスはローカルのMacで利用できます。',deleteApprovalNeeded:'削除の承認IDを入力してください。'},'zh-CN':{jobActions:'操作',revealRaster:'在Finder中显示光栅',deleteJob:'删除任务',deleteJobHelp:'从列表中删除此任务。原稿和光栅数据保留，由单独的保留期限清理处理。',deleteApproval:'删除审批ID',deleted:'任务已删除',revealed:'已在Finder中选中光栅',finderUnavailable:'仅可在本地Mac访问Finder。',deleteApprovalNeeded:'请输入删除审批ID。'}};
 for(const lang of Object.keys(words))Object.assign(texts[lang],words[lang]);
 let config=null,pending=null,busy=false;
 const dialog=$('delete-job-dialog');
 function reset(){config=null;pending=null;busy=false;dialog.close();$('delete-job-approval').value='';}
 async function load(s){config=null;try{const value=await json('/api/v1/console/actions',{},s);if(session===s)config=value;}catch(e){if(session===s)report(e);}if(session===s)renderJobs();}
 function button(job,action,path,enabled,handler){const b=document.createElement('button');b.type='button';b.className='icon'+(action==='delete'?' danger':'');b.dataset.jobAction=action;b.dataset.jobId=job.id;const label=t(action==='delete'?'deleteJob':'revealRaster');b.title=label;b.setAttribute('aria-label',label);b.disabled=!enabled||busy;b.innerHTML=`<svg viewBox="0 0 24 24"><path d="${path}"/></svg>`;b.onclick=handler;return b;}
 function cell(job){const cell=document.createElement('td'),group=document.createElement('div');group.className='job-actions';
  group.append(button(job,'reveal','M3 7V4h6l2 3h10v13H3z M12 10v7m-3-3 3 3 3-3',job.state==='COMPLETED'&&config?.finder,()=>reveal(job)));
  group.append(button(job,'delete','M3 6h18M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7',terminal(job.state)&&config?.delete,()=>confirm(job)));
  cell.append(group);return cell;
 }
 async function reveal(job){const s=session;if(!s||busy)return;busy=true;renderJobs();try{await request(`/api/v1/jobs/${job.id}/reveal`,{method:'POST'},s);if(s===session)notify('revealed');}catch(e){if(s===session)report(e);}finally{if(s===session){busy=false;renderJobs();}}}
 function confirm(job){if(!session||busy)return;pending={job,s:session};$('delete-job-name').textContent=job.manifest.name;$('delete-job-approval').value='';$('delete-job-approval-label').hidden=!config?.deleteApprovalRequired;$('delete-job-confirm').disabled=false;dialog.showModal();$('delete-job-close').focus();}
 $('delete-job-close').onclick=()=>{dialog.close();pending=null;};dialog.addEventListener('cancel',()=>{pending=null;});
 $('delete-job-form').onsubmit=async event=>{event.preventDefault();if(!pending||busy)return;const {job,s}=pending;if(s!==session)return;const headers={'x-jrip-expected-revision':String(job.revision)};
  if(config?.deleteApprovalRequired){const approval=$('delete-job-approval').value.trim();if(!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(approval)){notify('deleteApprovalNeeded');$('delete-job-approval').focus();return;}headers['x-jrip-approval-id']=approval;}
  busy=true;$('delete-job-confirm').disabled=true;
  try{await request(`/api/v1/jobs/${job.id}`,{method:'DELETE',headers},s);if(s!==session)return;dialog.close();pending=null;jobs=jobs.filter(j=>j.id!==job.id);if(selected?.id===job.id){selected=null;pages=[];pagesLoaded=false;selectionVersion++;resetPreview();renderPageOptions();renderDetails();}if(!jobs.length&&offset>0)offset=Math.max(0,offset-50);await refresh();if(s===session)notify('deleted');}
  catch(e){if(s===session){report(e);if(e.status===409)await refresh();}}
  finally{if(s===session){busy=false;$('delete-job-confirm').disabled=false;renderJobs();}}
 };
 return {cell,load,reset};
})();
translate();
