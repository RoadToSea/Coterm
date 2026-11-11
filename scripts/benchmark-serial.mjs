/** Explicit hardware benchmark through the opt-in serial diagnostic example.
 * Build with cargo build -p coterm-driver --example serial-diagnostic.
 * Measures UART/backend throughput; excludes the desktop in-process dispatch path.
 * Requires a released port and a logged-in Linux Shell with Python 3. The temporary
 * peer runs in memory, restores its tty after 30 seconds without a command, and exits.
 * Fails on missing/corrupt bytes, event gaps, unsupported speeds or unavailable ports.
 */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { createInterface } from "node:readline";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const root = fileURLToPath(new URL("../", import.meta.url));
const flags = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index], value = process.argv[index + 1];
  if (!['--port', '--baud', '--rates', '--bytes', '--repeats', '--output', '--diagnostic'].includes(key) || !value)
    throw new Error("Usage: node scripts/benchmark-serial.mjs --port PORT [--baud 115200] [--rates 115200,230400] [--bytes 65536] [--repeats 3] [--output FILE] [--diagnostic FILE]");
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
let received = Buffer.alloc(0), changed, connectionId = null, cursor = 0, gaps = 0;
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
  effect: operation === 'serial.status' ? 'read' : 'commit' });
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
  received = Buffer.alloc(0); cursor = 0; connectionId = null;
  await invoke('serial.connect');
  connectionId = (await invoke('serial.status')).connectionId;
}

// The board's interactive line editor is smaller than the tty canonical buffer.
const peer = `import os,sys,termios,tty,time,signal,select,hashlib
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
 out('\\nBENCH_SWITCH '+sys.argv[1]+'\\n'); termios.tcdrain(1); time.sleep(.6)
 a=termios.tcgetattr(fd); a[4]=a[5]=getattr(termios,'B'+sys.argv[1]); termios.tcsetattr(fd,termios.TCSANOW,a)
 out('BENCH_READY\\n')
 while True:
  signal.alarm(30)
  c=line().split()
  if c[0]=='Q': out('BENCH_BYE\\n'); termios.tcdrain(1); time.sleep(.6); break
  if c[0]=='P': out('BENCH_READY\\n'); continue
  n=int(c[1])
  if n<1 or n>131072: raise ValueError('invalid count')
  data=b''.join(hashlib.sha256(str(i).encode()).hexdigest().encode() for i in range((n+63)//64))[:n]
  if c[0]=='R':
   out('BENCH_BEGIN\\n'); out(data); termios.tcdrain(1); out('\\nBENCH_END\\n')
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
const payload = Buffer.from(Array.from({ length: Math.ceil(bytes / 64) }, (_, index) =>
  createHash('sha256').update(String(index)).digest('hex')).join('').slice(0, bytes));
const hash = createHash('sha256').update(payload).digest('hex');
const report = { port, originalBaud: baud, framing: '8N1', mode: 'stream', bytes, repeats, chunkBytes: 16384,
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
      const initialGaps = gaps;
      for (let repeat = 0; repeat < repeats; repeat++) {
        await send(`R ${bytes}\n`);
        await until('BENCH_BEGIN\n');
        const start = performance.now();
        const data = await read(bytes);
        const milliseconds = performance.now() - start;
        assert.deepEqual(data, payload, 'device-to-host payload is corrupted');
        await until('\nBENCH_END\n');
        row.samples.push({ direction: 'rx', milliseconds, bytesPerSecond: bytes * 1000 / milliseconds, sha256: hash });
        await send(`W ${bytes}\n`);
        await until('BENCH_SEND\n');
        const txStart = performance.now();
        for (let offset = 0; offset < bytes; offset += 16384) await send(payload.subarray(offset, offset + 16384).toString());
        const reply = await until('\n');
        const txMs = performance.now() - txStart;
        const match = /^BENCH_OK (\d+) ([a-f0-9]+) ([\d.]+)\n$/.exec(reply);
        assert.ok(match, 'invalid device acknowledgement');
        assert.equal(Number(match[1]), bytes); assert.equal(match[2], hash, 'host-to-device checksum mismatch');
        row.samples.push({ direction: 'tx', milliseconds: txMs, bytesPerSecond: bytes * 1000 / txMs,
          deviceReceiveMilliseconds: Number(match[3]) * 1000, sha256: hash });
      }
      row.eventGaps = gaps - initialGaps;
      assert.equal(row.eventGaps, 0, 'serial.output events were dropped');
      row.verified = true;
      await send('Q\n'); await until('BENCH_BYE\n');
      if (rate !== baud) await reconnect(baud);
      await until('BENCH_RESTORED\r\n', 5000); activePeer = false;
      await until('# ', 5000);
      console.log(JSON.stringify(row));
    } catch (error) {
      row.verified = false; row.error = error.message;
      row.diagnosticTail = received.subarray(-800).toString();
      console.log(JSON.stringify({ baud: rate, error: error.message, tail: row.diagnosticTail }));
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
if (!report.rates.some(rate => rate.verified)) process.exitCode = 1;
