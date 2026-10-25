// Trusted test bundles only. This is a JRIP profile check, not general ACX certification.
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
const {manifest,preflight:p,accepted:a,result:r}=JSON.parse(readFileSync(process.argv[2],'utf8'));
function require(ok,message) { if(!ok) throw new Error(message); }
function canonical(v) {
 if(typeof v==='number') require(Number.isFinite(v)&&Math.abs(v)<=Number.MAX_SAFE_INTEGER,'unsafe number');
 if(Array.isArray(v)) return '['+v.map(canonical).join(',')+']';
 if(v&&typeof v==='object') return '{'+Object.keys(v).sort().map(k=>JSON.stringify(k)+':'+canonical(v[k])).join(',')+'}';
 return JSON.stringify(v);
}
const hash=v=>'sha256:'+createHash('sha256').update(canonical(v)).digest('hex');
const capability=manifest.capabilities.find(c=>c.id===p.capability);
require(capability && capability.extensions['org.jrip.print'].lifecycle.experimental,'missing experimental lifecycle');
require(p.profile==='org.jrip.settings-preflight-v2'&&p.digestProfile==='org.jrip.jcs-safe-v1','unsupported preflight');
const unsigned={...p}; delete unsigned.preflightDigest;
require(hash(unsigned)===p.preflightDigest&&hash(p.ticket)===p.requestHash,'preflight hash mismatch');
require(p.authorizationGranted===false&&p.documentVerified===false,'preflight claims execution');
require(p.authorizationEvidence.actor===p.actor&&p.authorizationEvidence.scopeCheck==='passed','invalid authority evidence');
require(a.status==='accepted'&&a.proof.profile==='org.jrip.acceptance-receipt-v1','not acceptance');
require(a.provider===manifest.provider.id&&r.provider===a.provider&&a.capability===p.capability&&r.capability===a.capability,'provider/capability mismatch');
require(a.requestHash===p.requestHash&&r.requestHash===a.requestHash,'request mismatch');
require(a.proof.preflightId===p.preflightId&&a.proof.preflightDigest===p.preflightDigest&&a.proof.actor===p.actor,'acceptance binding mismatch');
require(a.receiptId===a.proof.jobId&&a.proof.sourceSha256===p.ticket.document.sha256,'source/job mismatch');
require(canonical(a.proof.authorizationEvidence)===canonical(p.authorizationEvidence),'authorization mismatch');
require(a.proof.requestHashDigestProfile===p.digestProfile&&a.proof.preflightDigestProfile===p.digestProfile,'digest profile mismatch');
require(r.proof.profile==='org.jrip.result-receipt-v1'&&r.proof.digestProfile==='org.jrip.serde-json-v1','unsupported result');
require(r.proof.acceptanceReceiptId===a.receiptId&&r.proof.result.jobId===a.proof.jobId&&r.receiptId===a.receiptId+':result','result linkage mismatch');
require(r.proof.requestHashDigestProfile===p.digestProfile,'result request profile mismatch');
const outcome=r.proof.result;
require(outcome.sourceSha256===a.proof.sourceSha256&&outcome.completionMeaning==='raster_export','source/completion mismatch');
require(({COMPLETED:'succeeded',FAILED:'failed',CANCELLED:'cancelled'})[outcome.state]===r.status,'state/status mismatch');
// serde-json-v1 fixtures have string keys ordered by UTF-8; this profile's keys are ASCII,
// and these generated outcomes contain only safe integers, booleans, strings and nulls.
require(hash(outcome)===r.resultHash,'result hash mismatch');
if(r.status==='succeeded') require(outcome.artifactHashesVerified===true&&outcome.pageCount>0&&outcome.artifacts.length===outcome.pageCount,'missing successful artifacts');
console.log('JRIP experimental lifecycle bindings verified');
