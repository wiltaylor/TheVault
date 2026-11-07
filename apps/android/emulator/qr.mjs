// The QR code of check 10 (TV-67). It was made by `qrencode -o - -s 8 <text>`, which the box does not have.
// The qrcode package renders the same PNG in node, so the harness needs no tool on the box.

import QRCode from "qrcode";

/**
 * A PNG of the QR code for `text`, as a Buffer. `scale` is the pixels per module and `margin` the
 * quiet zone in modules; the defaults match `qrencode -s 8` (margin 4, error correction level L).
 */
export async function qrPng(text, { scale = 8, margin = 4 } = {}) {
  if (typeof text !== "string" || text.length === 0) throw new Error("qrPng: the text must be a non-empty string");
  return QRCode.toBuffer(text, { type: "png", scale, margin, errorCorrectionLevel: "L" });
}
