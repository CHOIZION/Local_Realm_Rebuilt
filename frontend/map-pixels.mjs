export function paintGridRect(rgba, cells, stride, palette, left, top, right, bottom) {
  // Both views use host byte order, so the four RGBA bytes survive the copy exactly.
  const pixels = new Uint32Array(rgba.buffer, rgba.byteOffset, rgba.byteLength / 4);
  for (let y = top; y <= bottom; y++) {
    const end = y * stride + right;
    for (let index = y * stride + left; index <= end; index++) {
      pixels[index] = palette[cells[index]];
    }
  }
}
