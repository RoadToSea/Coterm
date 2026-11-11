/** Explicit loopback throughput/loss test of the real TCP backend through the diagnostic adapter.
 * Build: cargo build -p coterm-driver --release --example network-diagnostic
 * No desktop, xterm or MCP is exercised. Payloads, queues and logs remain bounded.
 */
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { createHash, randomBytes } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay, setImmediate as yieldToIo } from "node:timers/promises";
import { once } from "node:events";
import { throughputFrame, FrameVerifier, FRAME_BYTES } from "./support/throughput-frames.mjs";
const root = fileURLToPath(new URL("../", import.meta.url)), flags = new Map();
for (let i = 2; i < process.argv.length; i += 2) {
    const key = process.argv[i], value = process.argv[i + 1];
    if (!["--seconds", "--rates", "--output", "--diagnostic"].includes(key) || !value)
        throw new Error("Usage: benchmark-tcp.mjs [--seconds 60] [--rates 1048576,5242880,10485760,unlimited] [--output FILE] [--diagnostic FILE]");
    flags.set(key, value);
}
const seconds = Number(flags.get("--seconds") ?? 60);
assert.ok(Number.isInteger(seconds) && seconds >= 1 && seconds <= 3600, "Invalid --seconds");
const rates = (flags.get("--rates") ?? "1048576,5242880,10485760,unlimited").split(",").map(value => value === "unlimited" ? 0 : Number(value));
assert.ok(rates.length <= 12 && rates.every(value => Number.isSafeInteger(value) && (value === 0 || value >= 65536) && value <= 1024 ** 3), "Invalid --rates");
const diagnostic = path.resolve(flags.get("--diagnostic") ?? path.join(root, "target/release/examples", `network-diagnostic${process.platform === "win32" ? ".exe" : ""}`));
const child = spawn(diagnostic, ["tcp"], { windowsHide: true, stdio: ["pipe", "pipe", "pipe"] });
const pending = new Map(), completed = new Map();
let sequence = 0, stderr = "", onOutput = () => { };
child.stderr.on("data", bytes => { stderr = (stderr + bytes).slice(-2048); });
const fail = error => { for (const item of pending.values()) {
    clearTimeout(item.timer);
    item.reject(error);
} pending.clear(); };
child.on("error", fail);
child.on("exit", code => fail(new Error(`Diagnostic exited ${code}: ${stderr}`)));
createInterface({ input: child.stdout }).on("line", line => {
    const message = JSON.parse(line);
    if (message.id != null) {
        const item = pending.get(message.id);
        if (!item)
            return;
        pending.delete(message.id);
        clearTimeout(item.timer);
        message.error ? item.reject(new Error(message.error.message)) : item.resolve(message.result);
    }
    else if (message.params?.type === "operation_completed" || message.params?.type === "operation_failed") {
        completed.set(message.params.operationId, message.params);
    }
    else if (message.params?.name === "tcp.output")
        onOutput(message.params.payload);
});
const rpc = (method, params) => new Promise((resolve, reject) => {
    const id = String(++sequence), timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, 15000);
    pending.set(id, { resolve, reject, timer });
    child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
});
const invoke = (id, operation, args = {}) => rpc("invoke", { operationId: `throughput-${sequence}`, driverId: "coterm-driver-tcp", driverInstanceId: id,
    operation: `tcp.${operation}`, args, source: "human", dispatch: { mode: "execute" }, effect: ["status", "read", "read_hex", "read_lines"].includes(operation) ? "read" : "commit" });
async function waitFor(predicate, timeout = 15000) {
    const deadline = performance.now() + timeout;
    while (!predicate()) {
        if (performance.now() > deadline)
            throw new Error("Throughput condition timed out");
        await delay(5);
    }
}
async function pace(start, bytes, rate) { if (!rate)
    return; const wait = bytes * 1000 / rate - (performance.now() - start); if (wait >= 1)
    await delay(wait); }
const report = { startedAt: new Date().toISOString(), path: "loopback TCP -> real Backend -> bounded notifications -> diagnostic JSON -> verified Node consumer",
    excludes: "desktop Runtime dispatch, WebView, xterm, MCP and approval", frameBytes: FRAME_BYTES, secondsPerRate: seconds, cases: [] };
let server, socket;
async function runCase(rate, index) {
    const id = `throughput-${index}`, nonce = randomBytes(8).toString("hex");
    const receiver = new FrameVerifier(nonce, "R"), peerReceiver = new FrameVerifier(nonce, "T");
    const sourceHash = createHash("sha256"), txHash = createHash("sha256");
    let sentFrames = 0, txFrames = 0, eventGaps = 0, cursor = 0, deliveredBytes = 0;
    server = net.createServer(peer => { socket = peer; peer.setNoDelay(true); peer.on("data", bytes => peerReceiver.feed(bytes)); peer.on("error", () => { }); });
    await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
    await rpc("create_instance", { instance: { driverId: "coterm-driver-tcp", driverInstanceId: id, sessionId: id, label: id, status: "ready" }, config: { host: "127.0.0.1", port: server.address().port, mode: "stream" } });
    const accepted = await invoke(id, "connect");
    await waitFor(() => completed.has(accepted._cotermDeferred));
    const connected = completed.get(accepted._cotermDeferred);
    completed.delete(accepted._cotermDeferred);
    if (connected.type !== "operation_completed")
        throw new Error(connected.error.message);
    const epoch = connected.result.connectionId;
    onOutput = payload => {
        if (payload.connectionId !== epoch)
            return;
        const skipped = payload.cursor - cursor - Math.ceil(payload.bytes / 1024);
        if (skipped > 0)
            eventGaps += skipped;
        cursor = payload.cursor;
        deliveredBytes += payload.bytes;
        receiver.feed(Buffer.from(payload.text));
    };
    const start = performance.now(), deadline = start + seconds * 1000;
    await Promise.all([
        (async () => {
            while (performance.now() < deadline) {
                const bytes = throughputFrame(nonce, "R", sentFrames);
                sourceHash.update(bytes);
                sentFrames++;
                if (!socket.write(bytes))
                    await once(socket, "drain");
                await pace(start, sentFrames * FRAME_BYTES, rate);
                if (sentFrames % 16 === 0)
                    await yieldToIo();
            }
        })(),
        (async () => {
            while (performance.now() < deadline) {
                const bytes = throughputFrame(nonce, "T", txFrames);
                txHash.update(bytes);
                await invoke(id, "write", { connectionId: epoch, text: bytes.toString("ascii"), lineEnding: "none" });
                txFrames++;
                await pace(start, txFrames * FRAME_BYTES, rate);
            }
        })(),
    ]);
    const activeSeconds = (performance.now() - start) / 1000;
    await waitFor(() => peerReceiver.bytes === txFrames * FRAME_BYTES);
    let status;
    for (let tries = 0; tries < 100; tries++) {
        status = await invoke(id, "status");
        if (status.rxBytes === sentFrames * FRAME_BYTES)
            break;
        await delay(20);
    }
    await delay(300);
    const rx = receiver.finish(sentFrames, sourceHash.digest("hex")), tx = peerReceiver.finish(txFrames, txHash.digest("hex"));
    const replay = await invoke(id, "read_hex", { cursor: 0 });
    const lines = await invoke(id, "read_lines", { lines: 1000 });
    const result = { offeredBytesPerSecondPerDirection: rate || "unlimited", activeSeconds, rxBytesPerSecond: sentFrames * FRAME_BYTES / activeSeconds,
        txBytesPerSecond: txFrames * FRAME_BYTES / activeSeconds, backendRxBytes: status.rxBytes, backendTxBytes: status.txBytes,
        eventDeliveredBytes: deliveredBytes, eventMissingChunks: eventGaps, receive: rx, send: tx,
        backendCountVerified: status.rxBytes === sentFrames * FRAME_BYTES && status.txBytes === txFrames * FRAME_BYTES,
        boundedHistory: { replayGap: replay.gap, readLinesDropped: lines.dropped }, nodeRssBytes: process.memoryUsage().rss };
    report.cases.push(result);
    console.log(JSON.stringify(result));
    onOutput = () => { };
    await invoke(id, "disconnect");
    await rpc("destroy_instance", { driverInstanceId: id });
    socket.destroy();
    await new Promise(resolve => server.close(resolve));
    server = null;
    socket = null;
}
try {
    await rpc("initialize", { interfaceMajor: 2, interfaceMinor: 0, requiredFeatures: ["instances", "backend-calls", "operation-catalog"], optionalFeatures: ["driver-events", "async-operations"], limits: { maxControlBytes: 1048576, maxDataChunkBytes: 65536, initialWindow: { chunks: 16, bytes: 1048576 } } });
    for (let i = 0; i < rates.length; i++) {
        console.log(`Testing full duplex ${rates[i] || "unlimited"} B/s per direction for ${seconds}s`);
        await runCase(rates[i], i);
    }
}
finally {
    socket?.destroy();
    if (server)
        await new Promise(resolve => server.close(resolve));
    await rpc("shutdown", null).catch(() => { });
    child.stdin.end();
    if (flags.has("--output")) {
        const output = path.resolve(flags.get("--output"));
        await mkdir(path.dirname(output), { recursive: true });
        await writeFile(output, JSON.stringify(report, null, 2) + "\n");
    }
}
if (report.cases.some(row => !row.backendCountVerified || !row.receive.verified || !row.send.verified))
    process.exitCode = 1;
