#!/usr/bin/env node
// @ts-check
// Prints an ffmpeg filter chain for burning or decoding the barcode.
//
// Exists so shell scripts consume the single geometry definition in
// lib/barcode.mjs instead of re-deriving it — a burner and decoder that drifted
// apart would fail silently and be miserable to diagnose.
//
// Takes no resolution: the geometry is expressed as fractions of the frame, so
// the filters are resolution-independent ffmpeg expressions.
import { burnFilter, decodeFilter, barcodeRect } from '../lib/barcode.mjs';

const [mode, widthArg, heightArg] = process.argv.slice(2);

if (mode === 'burn') process.stdout.write(burnFilter());
else if (mode === 'decode') process.stdout.write(decodeFilter());
else if (mode === 'rect') {
  const width = Number(widthArg);
  const height = Number(heightArg);
  if (!Number.isFinite(width) || !Number.isFinite(height)) {
    console.error('usage: barcode-filter.mjs rect <width> <height>');
    process.exit(2);
  }
  const r = barcodeRect(width, height);
  process.stdout.write(`${r.w}x${r.h}+${r.x}+${r.y} (cell ${r.cellW}px)`);
} else {
  console.error('usage: barcode-filter.mjs <burn|decode|rect [w h]>');
  process.exit(2);
}
