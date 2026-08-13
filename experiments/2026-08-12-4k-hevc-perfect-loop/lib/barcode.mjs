// @ts-check

/**
 * Binary frame-index barcode burned into the dex test card.
 *
 * PLACEMENT — centred, and exactly on the test card's grid.
 *
 * The card's grid was measured from a real frame: 50 px cells at 1080p, with
 * lines at x = 10 + 50k and y = 40 + 50k. The barcode is 18 cells of one grid
 * square each:
 *
 *     1080p:  900 x 50  at (510, 940)      510 + 450 = 960 -> centred
 *                                          510 = 10 + 10*50, 940 = 40 + 18*50 -> on grid
 *     2160p: 1800 x 100 at (1020, 1880)    same, scaled 2x
 *
 * It sits in the lower third, clear of the centre circle, the greyscale ramp,
 * the colour wheels and the bottom arrow — and, unlike the earlier top-edge
 * placement, it no longer covers the checkerboard border or the top marker.
 *
 * LAYOUT — 18 cells:
 *
 *   cell 0      sync WHITE - supplies the white reference level
 *   cell 1      sync BLACK - supplies the black reference level
 *   cell 2 + k  data bit k of the frame index, LSB at cell 2 (k = 0..15)
 *
 * Thresholding against the two sync cells rather than fixed levels is what lets
 * one decoder read a file, an HDMI capture card, and a camera pointed at a
 * screen: all three shift and compress the level range differently.
 *
 * 16 data bits = 65,536 frames, about 36 minutes at 30 fps.
 *
 * GEOMETRY IS EXPRESSED AS FRACTIONS OF THE FRAME, so both filters are built
 * from ffmpeg `iw`/`ih` expressions and neither the burner nor the decoder needs
 * to be told the resolution. That is why `capture.mjs` takes no --width/--height.
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
/** Minimum separation between the sync levels for a frame to be readable. */
export const SYNC_MIN_DELTA = 40;

/**
 * PLACEMENT — inside the card's own black label bar.
 *
 * The card carries a black bar in the lower third whose LEFT half is reserved
 * for this barcode and whose right half holds the human-readable label
 * ("4K 3840x2160@30fps"). Measured from the rendered 4K card, 2026-08-13:
 *
 *     black bar   x 921..2918   y 1880..1980
 *     label text  x 2075..2758
 *     free left   x 921..2074
 *
 * The barcode is inset into that free space with a margin, so the bar frames it
 * on all sides and it can never collide with the label. An earlier version was
 * centred on the frame and would have run straight through the text.
 */
/** Barcode left edge as a fraction of frame width (941/3840). */
export const BAR_X_FRAC = 941 / 3840;
/** Barcode width as a fraction of frame width (1098/3840 — 18 cells of 61px at 4K). */
export const BAR_W_FRAC = 1098 / 3840;
/** Barcode top edge as a fraction of frame height (1884/2160, inset into the bar). */
export const BAR_Y_FRAC = 1884 / 2160;
/** Barcode height as a fraction of frame height (92/2160, inset into the bar). */
export const BAR_H_FRAC = 92 / 2160;

/** ffmpeg expressions for the barcode rectangle. Resolution-independent. */
const EXPR = {
  w: `iw*${BAR_W_FRAC}`,
  h: `ih*${BAR_H_FRAC}`,
  x: `iw*${BAR_X_FRAC}`,
  y: `ih*${BAR_Y_FRAC}`,
  cellW: `iw*${BAR_W_FRAC}/${CELLS}`,
};

/**
 * Pixel rectangle of the barcode for a given frame size.
 * Provided for tests and diagnostics; the filters use expressions instead.
 *
 * @param {number} width
 * @param {number} height
 * @returns {{x: number, y: number, w: number, h: number, cellW: number}}
 */
export function barcodeRect(width, height) {
  const w = Math.round(width * BAR_W_FRAC);
  const h = Math.round(height * BAR_H_FRAC);
  return {
    x: Math.round(width * BAR_X_FRAC),
    y: Math.round(height * BAR_Y_FRAC),
    w,
    h,
    cellW: w / CELLS,
  };
}

/**
 * ffmpeg filter chain that burns the frame index into the barcode rectangle.
 *
 * Data cell 2+k is lit when bit k of the frame counter `n` is set; the
 * expression `bitand(floor(n/2^k),1)` is evaluated per frame by drawbox's
 * `enable` option.
 *
 * @returns {string}
 */
export function burnFilter() {
  const parts = [
    `drawbox=x=${EXPR.x}:y=${EXPR.y}:w=${EXPR.w}:h=${EXPR.h}:color=black@1:t=fill`,
    `drawbox=x=${EXPR.x}:y=${EXPR.y}:w=${EXPR.cellW}:h=${EXPR.h}:color=white@1:t=fill`,
  ];
  for (let k = 0; k < DATA_BITS; k++) {
    const cell = 2 + k;
    const pow = 2 ** k;
    parts.push(
      `drawbox=x=${EXPR.x}+${cell}*${EXPR.cellW}:y=${EXPR.y}:` +
      `w=${EXPR.cellW}:h=${EXPR.h}:color=white@1:t=fill:` +
      `enable='eq(bitand(floor(n/${pow})\\,1)\\,1)'`
    );
  }
  return parts.join(',');
}

/**
 * ffmpeg filter chain that reduces each frame to just the barcode rectangle,
 * downscaled so one decode frame is FRAME_BYTES regardless of source resolution.
 *
 * This is why decoding is cheap at 4K: the JavaScript side never sees a full
 * frame. `flags=area` averages each cell, which raises noise immunity rather
 * than lowering it — the cells are large uniform blocks.
 *
 * @returns {string}
 */
export function decodeFilter() {
  return `crop=${EXPR.w}:${EXPR.h}:${EXPR.x}:${EXPR.y},` +
    `scale=${DECODE_W}:${DECODE_H}:flags=area,format=gray`;
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
