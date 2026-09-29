import { mkdtempSync, openSync, closeSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { NativeStreamDecoder } from '../src/delivery-native-stream.js';
import { nativeCommandProgress } from '../src/delivery-native-progress.js';
import { runCommand } from '../src/delivery-process.js';
import { withOperationLock } from '../src/delivery-workspace.js';
import { NativeStderrTail } from '../src/delivery-native-stderr.js';

const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, {recursive:true,force:true}); });
const root = () => { const path=mkdtempSync(join(tmpdir(),'fabric-native-stream-')); roots.push(path); return path; };
const delta = (type: string, value: string) => JSON.stringify({type:'stream_event',event:{type:'content_block_delta',delta:{type,[type==='thinking_delta'?'thinking':'text']:value}}})+'\n';

it('counts only substantive deltas and keeps secrets out of durable progress/tail', () => {
  const directory=root(); let time=0;
  const io=nativeCommandProgress({directory,taskId:'test-task',stage:'repair',host:'claude-code',warnMs:120,cancelMs:300,now:()=>time});
  io.start(123);
  const secret='Bearer top-secret-password';
  io.stdout(Buffer.from(delta('text_delta',secret)+delta('thinking_delta',secret)));
  io.stderr(Buffer.from(secret));
  io.stdout(Buffer.from(JSON.stringify({type:'stream_event',event:{type:'content_block_start',index:0,content_block:{type:'tool_use',id:'tool-1',name:secret,input:secret}}})+'\n'));
  io.stdout(Buffer.from(JSON.stringify({type:'user',message:{content:[{type:'tool_result',tool_use_id:'tool-1',content:secret}]}})+'\n'));
  time=120; expect(io.tick()).toBeUndefined();
  time=299; io.stdout(Buffer.from('{"type":"heartbeat"}\nnot-json\n')); io.stderr(Buffer.from(secret));
  expect(io.tick()).toBeUndefined();
  time=300; expect(io.tick()).toBe('native-inactivity');
  expect(io.tick()).toBe('native-inactivity'); io.finish();
  const progress=readFileSync(join(directory,'progress.jsonl'),'utf8');
  expect(progress).not.toContain(secret); expect(readFileSync(join(directory,'native-tail.jsonl'),'utf8')).not.toContain(secret);
  const rows=progress.trim().split('\n').map(s=>JSON.parse(s));
  expect(rows.map(r=>r.event)).toContain('inactivity-warning'); expect(rows.map(r=>r.event)).toContain('inactivity-cancel');
  expect(rows.filter(r=>r.event==='inactivity-cancel')).toHaveLength(1);
  expect(rows.at(-1)).toMatchObject({pid:123,textBytes:Buffer.byteLength(secret),reasoningBytes:Buffer.byteLength(secret),toolStarts:1,toolResults:1});
});

it('recognized StructuredOutput and tool deltas refresh activity, unframed bytes do not', () => {
  const directory=root(); let time=0;
  const io=nativeCommandProgress({directory,taskId:'structured',stage:'review',host:'claude-code',warnMs:120,cancelMs:300,now:()=>time});
  io.start(123);
  const start=(name:string,index:number)=>JSON.stringify({type:'stream_event',event:{type:'content_block_start',index,content_block:{type:'tool_use',id:'tool-'+index,name}}})+'\n';
  const chunk=(index:number)=>JSON.stringify({type:'stream_event',event:{type:'content_block_delta',index,delta:{type:'input_json_delta',partial_json:'secret-payload'}}})+'\n';
  io.stdout(Buffer.from(start('StructuredOutput',0)));
  for(let i=0;i<10;i++){time+=200;const bytes=Buffer.from(chunk(0));io.stdout(bytes.subarray(0,37));io.stdout(bytes.subarray(37));expect(io.tick()).toBeUndefined();}
  io.stdout(Buffer.from(start('Read',1)));time+=200;io.stdout(Buffer.from(chunk(1)));expect(io.tick()).toBeUndefined();
  time+=300;io.stdout(Buffer.from(chunk(99)));
  expect(io.tick()).toBe('native-inactivity');io.finish();
  const text=readFileSync(join(directory,'progress.jsonl'),'utf8');expect(text).not.toContain('secret-payload');
  expect(JSON.parse(text.trim().split('\n').at(-1)!)).toMatchObject({structuredBytes:140,toolBytes:14});
});

it('redacts complete stderr lines across chunks before bounded tail retention', () => {
  const stderr=new NativeStderrTail({ANTHROPIC_AUTH_TOKEN:'super-secret-value',SECRET_MULTILINE:'first-secret\nsecond-secret'});
  for(const chunk of ['authorization: Bearer super-', 'secret-value\n', 'password=unknown-secret\n', '-----BEGIN PRIVATE KEY-----\nkey-body\n-----END PRIVATE KEY-----\n',
    'first-secret\nsecond-secret\n', 'x'.repeat(9000)+' super-secret-value\n', 'configured model unavailable\n']) stderr.push(Buffer.from(chunk));
  const text=stderr.finish();expect(Buffer.byteLength(text)).toBeLessThanOrEqual(8192);
  for(const secret of ['super-secret-value','unknown-secret','key-body','first-secret','second-secret'])expect(text).not.toContain(secret);
  expect(text).toContain('configured model unavailable');
});

it('missing final Claude envelope is explicit and oversize stderr is safely withheld', () => {
  const decoder=new NativeStreamDecoder('claude-code',()=>{},()=>{});decoder.finish();expect(decoder.error()).toBe('native-stream-no-result');
  const stderr=new NativeStderrTail({});stderr.push(Buffer.from('password='+ 's'.repeat(100_000)+'\nconfigured model unavailable\n'));
  const text=stderr.finish();expect(text).toContain('withheld');expect(text).toContain('stderr diagnostics suppressed');
});

it('ambiguous PEM mentions and oversize key content stay suppressed without arbitrary release', () => {
  const quoted=new NativeStderrTail({});
  quoted.push(Buffer.from('missing -----BEGIN PRIVATE KEY----- header\nconfigured model unavailable\n'));
  expect(quoted.finish()).toContain('stderr diagnostics suppressed');
  const key=new NativeStderrTail({});
  key.push(Buffer.from('-----BEGIN PRIVATE KEY-----'+'s'.repeat(70000)+'-----END PRIVATE KEY-----\nprivate-continuation\n'));
  const text=key.finish();expect(text).toContain('stderr diagnostics suppressed');
  expect(text).not.toContain('private-continuation');expect(text).not.toContain('s'.repeat(100));
});

it('prefixed and collapsed PEM blocks redact bodies and release only at explicit END', () => {
  for(const pem of ['ssh key: -----BEGIN OPENSSH PRIVATE KEY-----\nprivate-body\nlog: -----END OPENSSH PRIVATE KEY-----',
    '-----BEGIN PRIVATE KEY-----private-body-----END PRIVATE KEY-----']) {
    const tail=new NativeStderrTail({});tail.push(Buffer.from(pem+'\nconfigured model unavailable\n'));
    const text=tail.finish();expect(text).not.toContain('private-body');expect(text).toContain('configured model unavailable');
  }
  const io=nativeCommandProgress({directory:root(),taskId:'separate-notices',stage:'review',host:'codex'});
  io.stderr(Buffer.from('-----BEGIN PRI'));
  io.stdout(Buffer.from('{"type":"error","message":"Reconnecting 1/5"}\n'));
  io.stderr(Buffer.from('VATE KEY-----\nprivate-body\n-----END PRIVATE KEY-----\n'));
  const output=io.finish();expect(output.stderr).not.toContain('private-body');expect(output.stderr).toContain('Reconnecting 1/5');
});

it('discarded marker prefixes and ordered repeated/PGP markers cannot release private bodies', () => {
  const overflow=new NativeStderrTail({});
  overflow.push(Buffer.from('x'.repeat(70000)));overflow.push(Buffer.from('-----BEGIN PRIVATE KEY-----\nprivate-body\n'));
  expect(overflow.finish()).not.toContain('private-body');
  for(const header of ['-----BEGIN PRIVATE KEY-----a-----END PRIVATE KEY----- -----BEGIN PRIVATE KEY-----b',
    '-----BEGIN PGP PRIVATE KEY BLOCK-----']) {
    const tail=new NativeStderrTail({});tail.push(Buffer.from(header+'\nprivate-body\n'));
    const text=tail.finish();expect(text).not.toContain('private-body');expect(text).toContain('stderr diagnostics suppressed');
  }
});

it('Codex reconnect notices are diagnostic only; terminal turn.failed still fails', () => {
  let activity=0;const notices:string[]=[];
  const decoder=new NativeStreamDecoder('codex',()=>activity++,()=>{},undefined,message=>notices.push(message));
  decoder.stdout(Buffer.from('{"type":"error","message":"Reconnecting 1/5"}\n'));
  expect(decoder.error()).toBeUndefined();expect(activity).toBe(0);expect(notices).toEqual(['Reconnecting 1/5']);
  decoder.stdout(Buffer.from('{"type":"item.completed","item":{"id":"answer","type":"agent_message","text":"done"}}\n{"type":"turn.completed"}\n'));
  decoder.finish();expect(decoder.error()).toBeUndefined();expect(activity).toBe(1);
  const failed=new NativeStreamDecoder('codex',()=>{},()=>{});
  failed.stdout(Buffer.from('{"type":"turn.failed","error":{"message":"terminal failure"}}\n'));
  expect(failed.error()).toBe('native-stream-error');
});

it.each(['claude-code','codex'] as const)('genuine tool lifecycle refreshes %s, replayed events and stderr do not; notifies coordinator immediately', host => {
  const directory=root();let time=0;const notifications:Record<string,unknown>[]=[];
  const io=nativeCommandProgress({directory,taskId:'tool-events',stage:'review',host,now:()=>time,notify:e=>notifications.push(e)});
  const start=host==='codex'?{type:'item.started',item:{id:'call-1',type:'mcp_tool_call'}}:
    {type:'stream_event',event:{type:'content_block_start',index:0,content_block:{id:'call-1',type:'tool_use',name:'Read'}}};
  const result=host==='codex'?{type:'item.completed',item:{id:'call-1',type:'mcp_tool_call'}}:
    {type:'user',message:{content:[{type:'tool_result',tool_use_id:'call-1',content:'secret'}]}};
  io.start(44);time=100_000;io.stdout(Buffer.from(JSON.stringify(start)+'\n'));time=200_000;expect(io.tick()).toBeUndefined();
  io.stdout(Buffer.from(JSON.stringify(result)+'\n'));time=310_000;expect(io.tick()).toBeUndefined();expect(notifications).toHaveLength(0);
  time=320_000;io.tick();expect(notifications[0]).toMatchObject({type:'delivery-native-progress',event:'inactivity-warning',pid:44,taskId:'tool-events'});
  time=499_999;io.stdout(Buffer.from(JSON.stringify(result)+'\n'));io.stderr(Buffer.from('heartbeat secret'));expect(io.tick()).toBeUndefined();
  time=500_000;expect(io.tick()).toBe('native-inactivity');expect(notifications[1]?.event).toBe('inactivity-cancel');
  expect(JSON.stringify(notifications)).not.toContain('secret');
});

it('Codex 0.157.1 item updates count only actual text/output growth and preserve full stream digests', () => {
  let activity=0;const decoder=new NativeStreamDecoder('codex',()=>activity++,()=>{});
  const row={type:'item.updated',item:{id:'reason-1',type:'reasoning',text:'reasoning-secret'}};
  decoder.stdout(Buffer.from(JSON.stringify(row)+'\n'));decoder.stdout(Buffer.from(JSON.stringify(row)+'\n'));expect(activity).toBe(1);
  row.item.text+=' more';decoder.stdout(Buffer.from(JSON.stringify(row)+'\n'));expect(activity).toBe(2);
  decoder.finish();const output=JSON.parse(decoder.output());expect(output.reasoningBytes).toBe(21);expect(output.stdoutSha256).toMatch(/^[a-f0-9]{64}$/);
  expect(decoder.output()).not.toContain('reasoning-secret');
});

it('healthy fragmented Unicode reasoning/text resets inactivity; cumulative bytes do not exhaust output', () => {
  const directory=root(); let time=0;
  const io=nativeCommandProgress({directory,taskId:'healthy',stage:'implementation',host:'claude-code',warnMs:120,cancelMs:300,now:()=>time});
  io.start(456);
  const frame=Buffer.from(delta('thinking_delta','\u03bb'.repeat(1024)));
  for(let i=0;i<6000;i++) { time+=1; io.stdout(frame.subarray(0,150)); io.stdout(frame.subarray(150)); expect(io.tick()).toBeUndefined(); }
  io.stdout(Buffer.from('{"type":"result","is_error":false,"structured_output":{"outcome":"completed"}}\n'));
  const result=io.finish(); expect(JSON.parse(result.stdout).structured_output.outcome).toBe('completed');
  expect(result.error).toBeUndefined(); expect(readFileSync(join(directory,'native-tail.jsonl')).length).toBeLessThan(32_000);
});

it('native redacted thinking deltas refresh activity without pretending content bytes exist', () => {
  const directory=root(); let time=0;
  const io=nativeCommandProgress({directory,taskId:'redacted-thinking',stage:'implementation',host:'claude-code',warnMs:120,cancelMs:300,now:()=>time});
  const send=(event:unknown)=>io.stdout(Buffer.from(JSON.stringify({type:'stream_event',event})+'\n'));
  io.start(42);
  send({type:'content_block_start',index:0,content_block:{type:'thinking',thinking:''}});
  for(let i=0;i<5;i++) {
    time+=200;
    send({type:'content_block_delta',index:0,delta:{type:'thinking_delta',thinking:'',estimated_tokens:12}});
    expect(io.tick()).toBeUndefined();
  }
  send({type:'content_block_stop',index:0});
  time+=300;
  send({type:'content_block_delta',index:0,delta:{type:'thinking_delta',thinking:'',estimated_tokens:12}});
  io.stdout(Buffer.from('{"type":"system","subtype":"thinking_tokens","estimated_tokens":900,"estimated_tokens_delta":12}\n'));
  expect(io.tick()).toBe('native-inactivity');io.finish();
  const last=JSON.parse(readFileSync(join(directory,'progress.jsonl'),'utf8').trim().split('\n').at(-1)!);
  expect(last).toMatchObject({reasoningBytes:0,estimatedReasoningTokens:60});
});

it('invalid or unframed reasoning estimates never refresh activity', () => {
  let activity=0;const decoder=new NativeStreamDecoder('claude-code',()=>activity++,()=>{});
  const send=(event:unknown)=>decoder.stdout(Buffer.from(JSON.stringify({type:'stream_event',event})+'\n'));
  const estimate=(value:unknown,index=0)=>send({type:'content_block_delta',index,delta:{type:'thinking_delta',thinking:'',estimated_tokens:value}});
  estimate(12);
  send({type:'content_block_start',index:0,content_block:{type:'thinking'}});
  for(const value of [0,-1,null,'12',0.5,Number.MAX_SAFE_INTEGER+1])estimate(value);
  estimate(12,1);expect(activity).toBe(0);
  estimate(12);expect(activity).toBe(1);
  send({type:'message_stop'});estimate(12);expect(activity).toBe(1);
});

it('bounded frames fail closed, malformed/empty/tool argument chunks cannot refresh activity', () => {
  let activity=0; const decoder=new NativeStreamDecoder('claude-code',()=>activity++,()=>{},128);
  decoder.stdout(Buffer.from(delta('text_delta','')));
  decoder.stdout(Buffer.from('{"type":"ping"}\n'));
  decoder.stdout(Buffer.from('x'.repeat(200)+'\n'));
  decoder.finish(); expect(activity).toBe(0); expect(decoder.error()).toBe('native-stream-frame-limit');
});

async function execute(script: string, streaming: boolean, duration=1000, host: 'claude-code' | 'codex' = 'claude-code') {
  const directory=root(),outPath=join(directory,'stdout'),errPath=join(directory,'stderr');
  const out=openSync(outPath,'wx'),err=openSync(errPath,'wx');
  const observer=streaming?nativeCommandProgress({directory,taskId:'process-test',stage:'review',host,warnMs:100,cancelMs:250}):undefined;
  try {
    const result=await withOperationLock(directory,()=>runCommand([process.execPath,'-e',script],directory,{PATH:process.env.PATH??''},out,err,directory,
      {timeoutMs:duration,maxOutputBytes:128},undefined,undefined,observer));
    return {result,directory,stdout:readFileSync(outPath,'utf8'),stderr:readFileSync(errPath,'utf8')};
  } finally {closeSync(out);closeSync(err);}
}

it.each(['claude-code','codex'] as const)('%s heartbeat-only fake child cancels on inactivity, not subscription or cumulative output', async host => {
  const {result,directory}=await execute('setInterval(()=>{process.stdout.write(JSON.stringify({type:"ping",data:"x".repeat(1000)})+"\\n");process.stderr.write("secret-warning\\n")},10)',true,1000,host);
  expect(result.error).toBe('native-inactivity');
  expect(readFileSync(join(directory,'progress.jsonl'),'utf8')).toContain('inactivity-warning');
  expect(readFileSync(join(directory,'progress.jsonl'),'utf8')).not.toContain('secret-warning');
});

it('healthy fake stream exceeds old cumulative ceiling and returns complete final result', async () => {
  const script='let i=0;const t=setInterval(()=>{process.stdout.write('+JSON.stringify(delta('text_delta','x'.repeat(500)))+');if(++i===12){clearInterval(t);process.stdout.write(JSON.stringify({type:"result",structured_output:{ok:true}})+"\\n")}},30)';
  const {result,stdout}=await execute(script,true,2000);
  expect(result.exitCode).toBe(0); expect(result.error).toBeUndefined(); expect(JSON.parse(stdout).structured_output.ok).toBe(true);
});

it('fake native empty-thinking estimates survive longer than the idle deadline', async () => {
  const start={type:'stream_event',event:{type:'content_block_start',index:0,content_block:{type:'thinking',thinking:''}}};
  const frame={type:'stream_event',event:{type:'content_block_delta',index:0,delta:{type:'thinking_delta',thinking:'',estimated_tokens:8}}};
  const script='process.stdout.write('+JSON.stringify(JSON.stringify(start)+'\n')+');let i=0;const t=setInterval(()=>{process.stdout.write('+JSON.stringify(JSON.stringify(frame)+'\n')+');if(++i===12){clearInterval(t);process.stdout.write(JSON.stringify({type:"result",structured_output:{ok:true}})+"\\n")}},30)';
  const {result,stdout}=await execute(script,true,2000);
  expect(result.exitCode).toBe(0);expect(result.error).toBeUndefined();expect(JSON.parse(stdout).structured_output.ok).toBe(true);
});

it('deterministic checks retain combined output ceiling', async () => {
  const {result,stdout,stderr}=await execute('process.stdout.write("x".repeat(10000))',false);
  expect(result.error).toBe('check-output-limit'); expect(Buffer.byteLength(stdout)+Buffer.byteLength(stderr)).toBeLessThanOrEqual(128);
});

it('inactivity drains inherited-pipe descendants using existing process group ownership', async () => {
  const script='const{spawn}=require("node:child_process");const{writeFileSync}=require("node:fs");const child=spawn(process.execPath,["-e","setInterval(()=>{},1000)"],{stdio:"inherit"});writeFileSync("child.pid",String(child.pid));setInterval(()=>process.stdout.write("{\\"type\\":\\"ping\\"}\\n"),20)';
  const {result,directory}=await execute(script,true,2000);
  expect(result.error).toBe('native-inactivity'); expect(existsSync(join(directory,'child.pid'))).toBe(true);
  const pid=Number(readFileSync(join(directory,'child.pid'),'utf8'));
  let live=false;try{process.kill(pid,0);live=true;}catch{}
  // A reaped or exited zombie cannot hold pipes or keep working.
  if(live)expect(readFileSync(`/proc/${pid}/stat`,'utf8').split(') ')[1]?.[0]).toBe('Z');
});
