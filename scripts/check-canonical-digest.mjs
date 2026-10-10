// Independent JavaScript reference check for valid structured JSON values.
// Duplicate source properties must be rejected by a strict parser before this stage.
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
const [binary, file] = process.argv.slice(2);
if (!binary || !file) throw new Error('usage: node check-canonical-digest.mjs JRIP_DIGEST INPUT.json');
function canonical(value, depth=0) {
  if (depth > 64) throw new Error('depth exceeded');
  if (typeof value === 'number' && (!Number.isFinite(value) || Math.abs(value) > Number.MAX_SAFE_INTEGER)) throw new Error('number out of range');
  if (Array.isArray(value)) return '['+value.map(v=>canonical(v,depth+1)).join(',')+']';
  if (value && typeof value === 'object') return '{'+Object.keys(value).sort().map(k=>JSON.stringify(k)+':'+canonical(value[k],depth+1)).join(',')+'}';
  return JSON.stringify(value);
}
const expected = 'sha256:'+createHash('sha256').update(canonical(JSON.parse(readFileSync(file,'utf8')))).digest('hex');
const actual = execFileSync(binary,[file],{encoding:'utf8'}).trim();
if (actual !== expected) throw new Error(`Rust/JavaScript mismatch: ${actual} != ${expected}`);
console.log('Rust/JavaScript canonical digest matched');
