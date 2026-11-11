import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { FrameVerifier, throughputFrame, FRAME_BYTES } from "../scripts/support/throughput-frames.mjs";
const source = Buffer.concat([0, 1, 2].map(index => throughputFrame("run", "R", index)));
const digest = createHash("sha256").update(source).digest("hex");
test("throughput verifier accepts arbitrary fragmentation and rejects loss, duplicates, order and corruption", () => {
    const intact = new FrameVerifier("run", "R");
    for (let offset = 0; offset < source.length; offset += 317)
        intact.feed(source.subarray(offset, offset + 317));
    assert.equal(intact.finish(3, digest).verified, true);
    const corrupted = Buffer.from(source);
    corrupted[100] ^= 1;
    for (const bytes of [
        source.subarray(0, FRAME_BYTES * 2),
        Buffer.concat([source.subarray(0, 117), source.subarray(118)]),
        Buffer.concat([source.subarray(0, FRAME_BYTES), source]),
        Buffer.concat([throughputFrame("run", "R", 1), throughputFrame("run", "R", 0), throughputFrame("run", "R", 2)]),
        corrupted,
        Buffer.concat([0, 1, 2].map(index => throughputFrame("stale-run", "R", index))),
        Buffer.concat([0, 1, 2].map(index => throughputFrame("run", "T", index))),
    ]) {
        const verifier = new FrameVerifier("run", "R");
        verifier.feed(bytes);
        assert.equal(verifier.finish(3, digest).verified, false);
    }
});
test("throughput verifier bounds an unterminated corrupt stream", () => {
    const verifier = new FrameVerifier("run", "R");
    for (let i = 0; i < 100; i++)
        verifier.feed(Buffer.alloc(4096, 65));
    assert.ok(verifier.pending.length <= FRAME_BYTES * 2);
    assert.ok(verifier.corruptFrames > 0);
});
