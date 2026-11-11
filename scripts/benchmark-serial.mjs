/** Explicit hardware benchmark through the opt-in serial diagnostic example.
 * Build with cargo build -p coterm-driver --example serial-diagnostic.
 * Measures UART/backend throughput; excludes the desktop in-process dispatch path.
 * --seconds keeps verified transfers running for at least that duration per baud.
 * --transfer-mode selects alternating transfers (default) or simultaneous full duplex.
 * Requires a released port and a logged-in Linux Shell with Python 3. The temporary
 * peer runs in memory, restores its tty after 30 seconds without a command, and exits.
 * Fails on missing/corrupt bytes, event gaps, unsupported speeds or unavailable ports.
 */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { createInterface } from "node:readline";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const root = fileURLToPath(new URL("../", import.meta.url));
const flags = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index], value = process.argv[index + 1];
  if (!['--port', '--baud', '--rates', '--bytes', '--repeats', '--seconds', '--transfer-mode', '--output', '--diagnostic'].includes(key) || !value)
    throw new Error("Usage: node scripts/benchmark-serial.mjs --port PORT [--baud 115200] [--rates 115200,230400] [--bytes 65536] [--repeats 3] [--seconds 600] [--transfer-mode alternating|duplex] [--output FILE] [--diagnostic FILE]");
  flags.set(key, value);
}
const port = flags.get('--port');
if (!port) throw new Error('--port is required; this benchmark writes to the selected device');
const integer = (name, fallback, min, max) => {
  const value = Number(flags.get(name) ?? fallback);
  assert.ok(Number.isSafeInteger(value) && value >= min && value <= max, `invalid ${name}`);
  return value;
};
const baud = integer('--baud', 115200, 1, 4_000_000);
const bytes = integer('--bytes', 65536, 1024, 131072);
const repeats = integer('--repeats', 3, 1, 10);
const seconds = integer('--seconds', 0, 0, 3600);
const transferMode = flags.get('--transfer-mode') ?? 'alternating';
assert.ok(['alternating', 'duplex'].includes(transferMode), 'invalid --transfer-mode');
const rates = (flags.get('--rates') ?? String(baud)).split(',').map(Number);
assert.ok(rates.length <= 12 && rates.every(rate => Number.isSafeInteger(rate) && rate >= 9600 && rate <= 4_000_000), 'invalid --rates');
assert.ok(bytes * 10 / Math.min(...rates) < 20, 'reduce --bytes: each transfer must finish before the peer watchdog');
const diagnostic = path.resolve(flags.get('--diagnostic') ?? path.join(root, 'target/debug/examples', `serial-diagnostic${process.platform === 'win32' ? '.exe' : ''}`));
const child = spawn(diagnostic, [], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
const pending = new Map();
let sequence = 0, stderr = '';
child.stderr.on('data', data => { stderr = (stderr + data).slice(-4096); });
child.on('error', error => { for (const item of pending.values()) item.reject(error); });
child.on('exit', code => { for (const item of pending.values()) item.reject(new Error(`Diagnostic exited ${code}: ${stderr}`)); });
let received = Buffer.alloc(0), changed, connectionId = null, cursor = 0, gaps = 0, eventBytes = 0;
createInterface({ input: child.stdout }).on('line', line => {
  const message = JSON.parse(line);
  if (message.id != null) {
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    clearTimeout(request.timer);
    if (message.error) request.reject(new Error(message.error.message)); else request.resolve(message.result);
  } else if (message.params?.name === 'serial.output') {
    const data = message.params.payload;
    if (connectionId && data.connectionId !== connectionId) return;
    if (data.cursor !== cursor + 1) gaps++;
    cursor = data.cursor;
    eventBytes += data.bytes;
    received = Buffer.concat([received, Buffer.from(data.text, 'utf8')]);
    if (received.length > 1024 * 1024) throw new Error('Benchmark receive buffer exceeded 1 MiB');
    changed?.();
  }
});
function rpc(method, params) {
  return new Promise((resolve, reject) => {
    const id = String(++sequence);
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, 20000);
    pending.set(id, { resolve, reject, timer });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
}
const invoke = (operation, args = {}) => rpc('invoke', { driverId: 'coterm-driver-serial', driverInstanceId: 'benchmark',
  operationId: `bench-${sequence}`, operation, args, source: 'human', dispatch: { mode: 'execute' },
  effect: ['serial.status', 'serial.read_hex'].includes(operation) ? 'read' : 'commit' });
const send = text => invoke('serial.write', { text, lineEnding: 'none' });
async function waitFor(predicate, timeout = 20000) {
  const deadline = performance.now() + timeout;
  while (!predicate()) {
    const remaining = deadline - performance.now();
    if (remaining <= 0) throw new Error('Timed out waiting for benchmark peer');
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => { changed = null; reject(new Error('Timed out waiting for benchmark peer')); }, remaining);
      changed = () => { clearTimeout(timer); changed = null; resolve(); };
      if (predicate()) changed();
    });
  }
}
async function until(marker, timeout) {
  const target = Buffer.from(marker);
  await waitFor(() => received.indexOf(target) >= 0, timeout);
  const index = received.indexOf(target) + target.length;
  const result = received.subarray(0, index).toString();
  received = received.subarray(index);
  return result;
}
async function read(count) {
  await waitFor(() => received.length >= count);
  const result = received.subarray(0, count);
  received = received.subarray(count);
  return result;
}
async function reconnect(rate) {
  await invoke('serial.disconnect');
  await invoke('serial.configure', { baudRate: rate });
  received = Buffer.alloc(0); cursor = 0; eventBytes = 0; connectionId = null;
  await invoke('serial.connect');
  connectionId = (await invoke('serial.status')).connectionId;
}

// The board's interactive line editor is smaller than the tty canonical buffer.
const peer = `import os,sys,termios,tty,time,signal,select,hashlib,fcntl,struct,json,threading
fd=0
old=termios.tcgetattr(fd)
def expired(a,b): raise TimeoutError('benchmark watchdog')
signal.signal(signal.SIGALRM,expired)
def out(s):
 b=s.encode() if isinstance(s,str) else s
 while b:
  n=os.write(1,b); b=b[n:]
def line():
 b=b''
 while not b.endswith(b'\\n'):
  c=os.read(fd,1)
  if not c: raise EOFError()
  b+=c
  if len(b)>64: raise ValueError('command too long')
 return b.decode().strip()
try:
 signal.alarm(30)
 tty.setraw(fd)
 a=termios.tcgetattr(fd); a[0]&=~(termios.IXON|termios.IXOFF|termios.IXANY); termios.tcsetattr(fd,termios.TCSANOW,a)
 out('\\nBENCH_SWITCH '+sys.argv[1]+'\\n'); termios.tcdrain(1); time.sleep(.6)
 a=termios.tcgetattr(fd); a[4]=a[5]=getattr(termios,'B'+sys.argv[1]); termios.tcsetattr(fd,termios.TCSANOW,a)
 out('BENCH_READY\\n')
 while True:
  signal.alarm(30)
  c=line().split()
  if c[0]=='Q': out('BENCH_BYE\\n'); termios.tcdrain(1); time.sleep(.6); break
  if c[0]=='P': out('BENCH_READY\\n'); continue
  if c[0]=='I':
   try:
    v=struct.unpack('20i',fcntl.ioctl(fd,0x545d,bytes(80)))
    result=dict(zip(['cts','dsr','rng','dcd','rx','tx','frame','overrun','parity','brk','bufferOverrun'],v))
   except OSError as e: result={'unavailable':str(e)}
   result['inputFlags']=termios.tcgetattr(fd)[0]; result['originalInputFlags']=old[0]
   result['softwareFlowControl']=bool(result['inputFlags']&(termios.IXON|termios.IXOFF|termios.IXANY))
   out('BENCH_IOCOUNTS '+json.dumps(result)+'\\n'); continue
  n=int(c[1])
  if n<1 or n>131072: raise ValueError('invalid count')
  data=b''.join(hashlib.sha256((c[2]+':'+str(i)).encode()).hexdigest().encode() for i in range((n+63)//64))[:n]
  if c[0]=='D':
   out('BENCH_BEGIN '+c[2]+'\\n'); termios.tcdrain(1)
   worker=threading.Thread(target=out,args=(data,),daemon=True); worker.start()
   chunks=[]; left=n; start=time.monotonic()
   while left:
    b=os.read(fd,min(left,16384))
    if not b: raise EOFError()
    chunks.append(b); left-=len(b)
   worker.join(); termios.tcdrain(1); out('\\nBENCH_END\\n')
   out('BENCH_OK '+str(n)+' '+hashlib.sha256(b''.join(chunks)).hexdigest()+' '+str(time.monotonic()-start)+'\\n')
  elif c[0]=='R':
   out('BENCH_BEGIN '+c[2]+'\\n'); out(data); termios.tcdrain(1); out('\\nBENCH_END\\n')
  elif c[0]=='W':
   out('BENCH_SEND\\n'); termios.tcdrain(1); chunks=[]; left=n; start=time.monotonic()
   while left:
    b=os.read(fd,min(left,16384))
    if not b: raise EOFError()
    chunks.append(b); left-=len(b)
   got=b''.join(chunks)
   out('BENCH_OK '+str(len(got))+' '+hashlib.sha256(got).hexdigest()+' '+str(time.monotonic()-start)+'\\n')
finally:
 signal.alarm(0); termios.tcsetattr(fd,termios.TCSANOW,old); out('\\nBENCH_RESTORED\\n')
`;
const encoded = Buffer.from(peer).toString('base64');
const nonce = randomBytes(4).toString('hex');
const payloadFor = seed => Buffer.from(Array.from({ length: Math.ceil(bytes / 64) }, (_, index) =>
  createHash('sha256').update(`${seed}:${index}`).digest('hex')).join('').slice(0, bytes));

async function replayFailedReceive(current) {
  if (current?.direction !== 'rx') return null;
  const chunks = [];
  let after = 0, expired = false, more = true;
  for (let page = 0; page < 1024 && more; page++) {
    const data = await invoke('serial.read_hex', { cursor: after });
    chunks.push(Buffer.from(data.hex, 'hex'));
    after = data.cursor; expired ||= data.gap; more = data.more;
  }
  const retained = Buffer.concat(chunks);
  const marker = Buffer.from(`BENCH_BEGIN ${current.seed}\n`);
  const begin = retained.lastIndexOf(marker);
  const start = begin + marker.length;
  const end = begin < 0 ? -1 : retained.indexOf(Buffer.from('\nBENCH_END\n'), start);
  const payload = begin < 0 || end < 0 ? Buffer.alloc(0) : retained.subarray(start, end);
  const expected = payloadFor(current.seed);
  let firstMismatch = 0;
  while (firstMismatch < Math.min(payload.length, expected.length) && payload[firstMismatch] === expected[firstMismatch]) firstMismatch++;
  const completeMarkers = begin >= 0 && end >= 0;
  return { retainedBytes: retained.length, expiredOlderHistory: expired, more,
    completeMarkers, payloadBytes: completeMarkers ? payload.length : null,
    sha256: completeMarkers ? createHash('sha256').update(payload).digest('hex') : null,
    missingBytes: completeMarkers ? expected.length - payload.length : null,
    firstMismatch: completeMarkers ? firstMismatch : null,
    actualWindow: completeMarkers ? payload.subarray(Math.max(0, firstMismatch - 8), firstMismatch + 32).toString('hex') : null,
    expectedWindow: completeMarkers ? expected.subarray(Math.max(0, firstMismatch - 8), firstMismatch + 32).toString('hex') : null,
    contiguousInsertion: completeMarkers ? payload.length > expected.length && payload.subarray(firstMismatch + payload.length - expected.length).equals(expected.subarray(firstMismatch)) : null,
    contiguousDeletion: completeMarkers ? expected.length > payload.length && payload.subarray(firstMismatch).equals(expected.subarray(firstMismatch + expected.length - payload.length)) : null,
    payloadVerified: completeMarkers && payload.equals(expected) };
}
async function peerCounters() {
  await send('I\n');
  await until('BENCH_IOCOUNTS ');
  return JSON.parse(await until('\n'));
}
async function duplexSample(row, cycle) {
  const rxSeed = `${nonce}r${cycle}`, rxPayload = payloadFor(rxSeed);
  const txPayload = payloadFor(`${nonce}t${cycle}`);
  row.current = { direction: 'rx', cycle, stage: 'duplex', seed: rxSeed };
  await send(`D ${bytes} ${rxSeed}\n`);
  await until(`BENCH_BEGIN ${rxSeed}\n`);
  const started = performance.now();
  await Promise.all([
    read(bytes).then(data => assert.ok(data.equals(rxPayload), `duplex device-to-host mismatch in cycle ${cycle}`)),
    (async () => {
      for (let offset = 0; offset < bytes; offset += 16384)
        await send(txPayload.subarray(offset, offset + 16384).toString());
    })(),
  ]);
  await until('\nBENCH_END\n');
  const reply = await until('\n');
  const match = /^BENCH_OK (\d+) ([a-f0-9]+) ([\d.]+)\n$/.exec(reply);
  assert.ok(match, 'invalid duplex device acknowledgement');
  assert.equal(Number(match[1]), bytes);
  assert.equal(match[2], createHash('sha256').update(txPayload).digest('hex'), 'duplex host-to-device checksum mismatch');
  const milliseconds = performance.now() - started;
  for (const [direction, payload] of [['rx', rxPayload], ['tx', txPayload]])
    row.samples.push({ direction, cycle, milliseconds, bytesPerSecond: bytes * 1000 / milliseconds,
      sha256: createHash('sha256').update(payload).digest('hex') });
}
const report = { startedAt:new Date().toISOString(), port, originalBaud: baud, framing: '8N1', mode: 'stream', bytes, repeats, chunkBytes: 16384,
  secondsPerRate: seconds, transferMode, payload: 'per-transfer nonce, direction and cycle; ordered SHA-256 blocks',
  path: 'physical UART -> serial diagnostic example -> JSON events -> Node consumer; reverse uses serial.write; excludes desktop dispatch',
  excludes: 'desktop rendering, MCP Bridge and approval', rates: [], restored: false };
let activePeer = false;
try {
  await rpc('initialize', { interfaceMajor: 2, interfaceMinor: 0,
    requiredFeatures: ['instances', 'backend-calls', 'operation-catalog'], optionalFeatures: ['driver-events'],
    limits: { maxControlBytes: 1048576, maxDataChunkBytes: 65536, initialWindow: { chunks: 16, bytes: 1048576 } } });
  await rpc('create_instance', { instance: { driverId: 'coterm-driver-serial', driverInstanceId: 'benchmark',
    sessionId: 'benchmark', label: 'Throughput benchmark', status: 'ready' },
    config: { port, baudRate: baud, dtr: false, rts: false, mode: 'stream' } });
  await invoke('serial.connect');
  connectionId = (await invoke('serial.status')).connectionId;
  await send('\r');
  await until('# ', 5000);
  for (const rate of rates) {
    const row = { baud: rate, theoreticalBytesPerSecond: rate / 10, samples: [] };
    let soakStarted = null;
    report.rates.push(row);
    console.log(`Testing ${rate} baud (${bytes} bytes, ${repeats} repeats per direction)`);
    try {
      received = Buffer.alloc(0);
      await send("COTERM_BENCH_PEER=''\r"); await until('# ', 5000);
      for (let offset = 0; offset < encoded.length; offset += 256) {
        await send(`COTERM_BENCH_PEER=\"$COTERM_BENCH_PEER\"'${encoded.slice(offset, offset + 256)}'\r`);
        await until('# ', 5000);
      }
      await send(`python3 -u -c "exec(__import__('base64').b64decode('$COTERM_BENCH_PEER'))" ${rate}; unset COTERM_BENCH_PEER\r`);
      activePeer = true;
      await until(`\nBENCH_SWITCH ${rate}\n`, 8000);
      if (rate !== baud) await reconnect(rate);
      await until('BENCH_READY\n', 5000);
      row.deviceCountersBefore = await peerCounters();
      assert.equal(row.deviceCountersBefore.softwareFlowControl, false, 'benchmark peer must disable software flow control');
      const initialGaps = gaps;
      row.initialGaps = initialGaps;
      soakStarted = performance.now();
      const progress = repeat => {
        if ((repeat + 1) % 10 === 0) console.log(JSON.stringify({ baud: rate, transferMode, cycles: repeat + 1,
          elapsedSeconds: (performance.now() - soakStarted) / 1000, verifiedBytesPerDirection: (repeat + 1) * bytes }));
      };
      for (let repeat = 0; repeat < repeats || performance.now() - soakStarted < seconds * 1000; repeat++) {
        if (transferMode === 'duplex') {
          await duplexSample(row, repeat);
          progress(repeat);
          continue;
        }
        const rxSeed = `${nonce}r${repeat}`, rxPayload = payloadFor(rxSeed);
        const rxHash = createHash('sha256').update(rxPayload).digest('hex');
        row.current = {direction:'rx',cycle:repeat,stage:'begin',seed:rxSeed};
        await send(`R ${bytes} ${rxSeed}\n`);
        await until(`BENCH_BEGIN ${rxSeed}\n`);
        row.current.stage = 'payload';
        const start = performance.now();
        const data = await read(bytes);
        const milliseconds = performance.now() - start;
        assert.ok(data.equals(rxPayload), `device-to-host payload is missing, duplicated, reordered or corrupted in cycle ${repeat}`);
        row.current.stage = 'end';
        await until('\nBENCH_END\n');
        row.samples.push({ direction: 'rx', cycle: repeat, milliseconds, bytesPerSecond: bytes * 1000 / milliseconds, sha256: rxHash });
        const txSeed = `${nonce}t${repeat}`, payload = payloadFor(txSeed);
        const hash = createHash('sha256').update(payload).digest('hex');
        row.current = {direction:'tx',cycle:repeat,stage:'ready',seed:txSeed};
        await send(`W ${bytes} ${txSeed}\n`);
        await until('BENCH_SEND\n');
        row.current.stage = 'payload';
        const txStart = performance.now();
        for (let offset = 0; offset < bytes; offset += 16384) await send(payload.subarray(offset, offset + 16384).toString());
        row.current.stage = 'acknowledgement';
        const reply = await until('\n');
        const txMs = performance.now() - txStart;
        const match = /^BENCH_OK (\d+) ([a-f0-9]+) ([\d.]+)\n$/.exec(reply);
        assert.ok(match, 'invalid device acknowledgement');
        assert.equal(Number(match[1]), bytes); assert.equal(match[2], hash, 'host-to-device checksum mismatch');
        row.samples.push({ direction: 'tx', cycle: repeat, milliseconds: txMs, bytesPerSecond: bytes * 1000 / txMs,
          deviceReceiveMilliseconds: Number(match[3]) * 1000, sha256: hash });
        progress(repeat);
      }
      row.elapsedSeconds = (performance.now() - soakStarted) / 1000;
      row.summary = Object.fromEntries(['rx','tx'].map(direction => {
        const samples = row.samples.filter(sample => sample.direction === direction);
        return [direction,{transfers:samples.length,verifiedBytes:samples.length*bytes,
          bytesPerSecond:samples.length*bytes*1000/samples.reduce((sum,sample)=>sum+sample.milliseconds,0),
          minBytesPerSecond:Math.min(...samples.map(sample=>sample.bytesPerSecond)),
          maxBytesPerSecond:Math.max(...samples.map(sample=>sample.bytesPerSecond))}];
      }));
      row.eventGaps = gaps - initialGaps;
      row.outputCursor = cursor;
      row.eventBytes = eventBytes;
      row.deviceCountersAfter = await peerCounters();
      delete row.current;
      assert.equal(row.eventGaps, 0, 'serial.output events were dropped');
      row.verified = true;
      await send('Q\n'); await until('BENCH_BYE\n');
      if (rate !== baud) await reconnect(baud);
      await until('BENCH_RESTORED\r\n', 5000); activePeer = false;
      await until('# ', 5000);
      console.log(JSON.stringify({...row,samples:undefined}));
    } catch (error) {
      row.verified = false; row.error = error.message;
      if (soakStarted !== null) row.elapsedSeconds = (performance.now() - soakStarted) / 1000;
      row.eventGaps = gaps - (row.initialGaps ?? gaps);
      row.failure = {current:row.current,bufferedBytes:received.length,eventBytes,cursor,
        backend:await invoke('serial.status').catch(() => null),containsEnd:received.includes(Buffer.from('\nBENCH_END\n'))};
      row.failure.replay = await replayFailedReceive(row.current).catch(error => ({error:error.message}));
      row.failure.deviceCounters = row.current?.direction === 'rx' && row.failure.replay?.completeMarkers
        ? await peerCounters().catch(error => ({error:error.message}))
        : { unavailable: 'receive completion not confirmed' };
      row.diagnosticTail = received.subarray(-800).toString();
      console.log(JSON.stringify({ baud: rate, error: error.message, eventGaps:row.eventGaps, failure:row.failure }));
      // Do not send Shell/control bytes at an unknown speed: let the peer's watchdog restore it.
      await delay(32000);
      await reconnect(baud); activePeer = false;
      await send('\r'); await until('# ', 5000);
    }
  }
  report.restored = true;
} finally {
  if (activePeer) {
    await delay(32000);
    await reconnect(baud).catch(() => {});
  }
  await invoke('serial.disconnect').catch(() => {});
  await rpc('shutdown', null).catch(() => {});
  child.stdin.end();
  if (flags.has('--output')) {
    const output = path.resolve(flags.get('--output'));
    await mkdir(path.dirname(output), { recursive: true });
    await writeFile(output, JSON.stringify(report, null, 2) + '\n');
  }
}
console.log(JSON.stringify({ restored: report.restored, passedRates: report.rates.filter(rate => rate.verified).map(rate => rate.baud) }));
if (!report.restored || report.rates.some(rate => !rate.verified)) process.exitCode = 1;
