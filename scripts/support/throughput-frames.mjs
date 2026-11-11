import { createHash } from "node:crypto";
export const FRAME_BYTES = 16384;
/**
 * @param {string} nonce Per-run identity.
 * @param {"R"|"T"} direction Stream direction.
 * @param {number} sequence Integer in 0..999999999999; invalid values throw Error.
 * @returns {Buffer} One fixed-size ASCII frame, including its LF terminator.
 */
export function throughputFrame(nonce, direction, sequence) {
    if (!Number.isSafeInteger(sequence) || sequence < 0 || sequence > 999999999999) {
        throw new Error("Invalid frame sequence");
    }
    const seed = createHash("sha256").update(`${nonce}:${direction}:${sequence}`).digest("hex");
    const frame = Buffer.alloc(FRAME_BYTES, seed);
    frame.write(`CTM1 ${direction} ${String(sequence).padStart(12, "0")} `, 0, "ascii");
    frame[FRAME_BYTES - 1] = 10;
    return frame;
}
/** Incremental fixed-frame verifier. Stores at most two partial frames between feed calls. */
export class FrameVerifier {
    /** @param {string} nonce Run identity. @param {"R"|"T"} direction Expected direction. */
    constructor(nonce, direction) {
        this.nonce = nonce;
        this.direction = direction;
        this.pending = "";
        this.bytes = 0;
        this.validFrames = 0;
        this.missingFrames = 0;
        this.duplicateOrReorderedFrames = 0;
        this.corruptFrames = 0;
        this.next = 0;
        this.hash = createHash("sha256");
        this.firstError = null;
    }
    /** @param {Buffer} bytes Next stream fragment. @returns {void} Malformed data is counted, not thrown. */
    feed(bytes) {
        this.bytes += bytes.length;
        this.hash.update(bytes);
        this.pending += bytes.toString("utf8");
        let end;
        while ((end = this.pending.indexOf("\n")) >= 0) {
            const line = this.pending.slice(0, end + 1);
            this.pending = this.pending.slice(end + 1);
            const match = /^CTM1 ([RT]) (\d{12}) /.exec(line);
            if (!match || match[1] !== this.direction || Buffer.byteLength(line) !== FRAME_BYTES) {
                this.corruptFrames++;
                this.firstError ??= "Incomplete or malformed frame";
                continue;
            }
            const sequence = Number(match[2]);
            if (!Buffer.from(line).equals(throughputFrame(this.nonce, this.direction, sequence))) {
                this.corruptFrames++;
                this.firstError ??= `Corrupt frame ${sequence}`;
                continue;
            }
            if (sequence > this.next)
                this.missingFrames += sequence - this.next;
            else if (sequence < this.next)
                this.duplicateOrReorderedFrames++;
            this.next = Math.max(this.next, sequence + 1);
            this.validFrames++;
        }
        if (this.pending.length > FRAME_BYTES * 2) {
            this.corruptFrames++;
            this.firstError ??= "Unterminated frame exceeds bounded buffer";
            this.pending = this.pending.slice(-FRAME_BYTES);
        }
    }
    /**
     * @param {number} expectedFrames Number actually sent by the producer.
     * @param {string} expectedDigest SHA-256 of the complete sent stream.
     * @returns {object} Verification and loss counters. Consumes the hash; call once per run.
     */
    finish(expectedFrames, expectedDigest) {
        const digest = this.hash.digest("hex");
        const missingFrames = this.missingFrames + Math.max(0, expectedFrames - this.next);
        return { bytes: this.bytes, expectedBytes: expectedFrames * FRAME_BYTES, validFrames: this.validFrames,
            missingFrames, duplicateOrReorderedFrames: this.duplicateOrReorderedFrames, corruptFrames: this.corruptFrames,
            partialBytes: Buffer.byteLength(this.pending), sha256: digest, expectedSha256: expectedDigest, firstError: this.firstError,
            verified: this.bytes === expectedFrames * FRAME_BYTES && digest === expectedDigest && missingFrames === 0
                && this.duplicateOrReorderedFrames === 0 && this.corruptFrames === 0 && this.pending.length === 0 };
    }
}
