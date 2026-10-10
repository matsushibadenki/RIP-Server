// Conformance test for trusted fixture JSON produced by Rust, not a general JSON parser.
// Production consumers must reject duplicate properties and invalid Unicode before verification.
import {readFileSync} from 'node:fs';
import {createHash,createPublicKey,verify} from 'node:crypto';
const [publicFile,receiptFile]=process.argv.slice(2);
const publicKey=readFileSync(publicFile);
const body=JSON.parse(readFileSync(receiptFile,'utf8'));
const s=body.signature;
if(publicKey.length!==32 || s.profile!=='org.jrip.receipt-ed25519-v2' || s.digestProfile!=='org.jrip.jcs-safe-v1' || s.algorithm!=='Ed25519' || s.encoding!=='hex' || s.publicKey!==publicKey.toString('hex') || s.keyId!=='sha256:'+createHash('sha256').update(publicKey).digest('hex') || !/^[0-9a-f]{128}$/.test(s.value) || body.proof.signed!==true) throw new Error('invalid signature metadata');
const signature=Buffer.from(s.value,'hex');
delete body.signature.value;
function canonical(v,depth=0) {
 if(depth>64) throw new Error('depth exceeded');
 if(typeof v==='number' && (!Number.isFinite(v) || Math.abs(v)>Number.MAX_SAFE_INTEGER)) throw new Error('number out of range');
 if(Array.isArray(v)) return '['+v.map(x=>canonical(x,depth+1)).join(',')+']';
 if(v && typeof v==='object') return '{'+Object.keys(v).sort().map(k=>JSON.stringify(k)+':'+canonical(v[k],depth+1)).join(',')+'}';
 return JSON.stringify(v);
}
const key=createPublicKey({key:Buffer.concat([Buffer.from('302a300506032b6570032100','hex'),publicKey]),format:'der',type:'spki'});
const message=Buffer.concat([Buffer.from(s.profile+'\0'),Buffer.from(canonical(body))]);
if(!verify(null,message,key,signature)) throw new Error('signature verification failed');
console.log('Rust signature verified independently by Node.js');
