// @ts-check

/**
 * Binary frame-index barcode burned into the top strip of every test-card frame.
 *
 * A strip across the top of the frame, `barHeight(height)` pixels tall, divided
 * into 18 equal-width cells:
 *
 *   cell 0      sync WHITE - supplies the white reference level
 *   cell 1      sync BLACK - supplies the black reference level
 *   cell 2 + k  data bit k of the frame index, LSB at cell 2 (k = 0..15)
 *
 * Thresholding against the two sync cells rather than against fixed levels is
 * what lets the same decoder read a file, an HDMI capture card, and a camera
 * pointed at a screen: all three shift and compress the level range differently.
 *
 * 16 data bits = 65,536 frames, about 36 minutes at 30 fps.
 */

/** Total cells across the strip: 2 sync + 16 data. */
export const CELLS = 18;
/** Frame-index width in bits. */
export const DATA_BITS = 16;
/** Pixels per cell after the decode downscale. */
export const CELL_PX = 4;
/** Decode-stage frame width in pixels. */
export const DECODE_W = CELLS * CELL_PX;
/** Decode-stage frame height in pixels. */
export const DECODE_H = 8;
/** Bytes per decode-stage gray frame. Independent of source resolution. */
export const FRAME_BYTES = DECODE_W * DECODE_H;
/** Minimum separation between the sync levels for a frame to be considered readable. */
export const SYNC_MIN_DELTA = 40;

/**
 * Height of the barcode strip for a given frame height.
 * Floored at 8 px so tiny test frames stay decodable after the area downscale.
 * @param {number} frameHeight
 * @returns {number}
 */
export function barHeight(frameHeight) {
  return Math.max(8, Math.round(frameHeight / 24));
}

/**
 * ffmpeg filter chain that burns the frame index into the top strip.
 *
 * Data cell 2+k is lit when bit k of the frame counter `n` is set; the
 * expression `bitand(floor(n/2^k),1)` is evaluated per frame by drawbox's
 * `enable` option.
 *
 * @param {number} frameHeight
 * @returns {string}
 */
export function burnFilter(frameHeight) {
  const h = barHeight(frameHeight);
  const parts = [
    `drawbox=x=0:y=0:w=iw:h=${h}:color=black@1:t=fill`,
    `drawbox=x=0:y=0:w=iw/${CELLS}:h=${h}:color=white@1:t=fill`,
  ];
  for (let k = 0; k < DATA_BITS; k++) {
    const cell = 2 + k;
    const pow = 2 ** k;
    parts.push(
      `drawbox=x=${cell}*iw/${CELLS}:y=0:w=iw/${CELLS}:h=${h}:color=white@1:t=fill:` +
      `enable='eq(bitand(floor(n/${pow})\\,1)\\,1)'`
    );
  }
  return parts.join(',');
}

/**
 * ffmpeg filter chain that reduces each frame to just the barcode strip,
 * downscaled so one decode frame is FRAME_BYTES regardless of source resolution.
 *
 * This is why decoding is cheap at 4K: the JavaScript side never sees a full
 * frame. `flags=area` averages each cell, which raises noise immunity rather
 * than lowering it — the cells are large uniform blocks.
 *
 * @param {number} frameHeight
 * @returns {string}
 */
export function decodeFilter(frameHeight) {
  return `crop=iw:${barHeight(frameHeight)}:0:0,scale=${DECODE_W}:${DECODE_H}:flags=area,format=gray`;
}

/**
 * Decode one downscaled gray frame to its embedded index.
 *
 * Returns null when the frame is unreadable — black, torn, or mistimed — so an
 * unreadable frame can never masquerade as a valid index. That distinction
 * matters: a black frame at the wrap is exactly the defect being hunted, and
 * silently decoding it as 0 would hide it.
 *
 * @param {Uint8Array} gray - exactly FRAME_BYTES of gray8, DECODE_W x DECODE_H
 * @returns {number | null}
 */
export function decodeFrame(gray) {
  if (gray.length !== FRAME_BYTES) throw new Error(`expected ${FRAME_BYTES} bytes, got ${gray.length}`);
  const row = Math.floor(DECODE_H / 2);
  /** @type {(cell: number) => number} */
  const sample = (cell) => gray[row * DECODE_W + cell * CELL_PX + Math.floor(CELL_PX / 2)];
  const white = sample(0);
  const black = sample(1);
  if (white - black < SYNC_MIN_DELTA) return null;
  const threshold = (white + black) / 2;
  let value = 0;
  for (let k = 0; k < DATA_BITS; k++) {
    if (sample(2 + k) > threshold) value |= 1 << k;
  }
  return value;
}
