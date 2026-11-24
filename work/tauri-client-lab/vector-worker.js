import { Bitmap, Potrace } from './potrace.mjs';
import { buildPaletteSync, utils } from './image-q.mjs';
import { gpuPosterize } from './vector-gpu.mjs';

const presets = {
  logo: { name: 'Logo / Warna Solid', colors: 5, turdSize: 5, optTolerance: 0.30, maxSide: 2200, minPixels: 12, gpuLevels: 4 },
  color: { name: 'Vektor Standar Berwarna', colors: 9, turdSize: 3, optTolerance: 0.22, maxSide: 1800, minPixels: 8, gpuLevels: 6 },
  detail: { name: 'Detail Warna Halus', colors: 14, turdSize: 2, optTolerance: 0.14, maxSide: 1600, minPixels: 5, gpuLevels: 7 },
};
const progress = (pct, text) => postMessage({ progress: { pct, text } });
const xml = value => value.replace(/[&<>"']/g, char => ({ '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&apos;' })[char]);

function colorLayers(raw, count) {
  const sample = [];
  const stride = Math.max(1, Math.ceil(raw.length / 4 / 70000));
  for (let i = 0; i < raw.length; i += 4 * stride) {
    if (raw[i + 3] >= 20) sample.push(raw[i], raw[i+1], raw[i+2], 255);
  }
  if (!sample.length && stride > 1) {
    for (let i = 0; i < raw.length && sample.length < 280000; i += 4) {
      if (raw[i + 3] >= 20) sample.push(raw[i], raw[i+1], raw[i+2], 255);
    }
  }
  if (!sample.length) throw Error('Tidak ada warna yang bisa ditrace');
  const image = utils.PointContainer.fromUint8Array(Uint8Array.from(sample), sample.length / 4, 1);
  const palette = buildPaletteSync([image], { colors: count, paletteQuantization: 'wuquant' });
  const colors = palette.getPointContainer().getPointArray().map(({ r, g, b }) => ({
    r, g, b, pixels: 0, color: '#' + [r,g,b].map(value => value.toString(16).padStart(2, '0')).join(''),
  }));
  const labels = new Uint8Array(raw.length / 4);
  labels.fill(255);
  const cache = new Map();
  for (let i = 0; i < raw.length; i += 4) {
    if (raw[i + 3] < 20) continue;
    const [r,g,b] = [raw[i],raw[i+1],raw[i+2]];
    const key = r * 65536 + g * 256 + b;
    let closest = cache.get(key);
    if (closest === undefined) {
      let distance = Infinity;
      for (let index = 0; index < colors.length; index++) {
        const color = colors[index];
        const delta = (color.r-r)**2 + (color.g-g)**2 + (color.b-b)**2;
        if (delta < distance) { distance = delta; closest = index; }
      }
      if (cache.size < 100000) cache.set(key, closest);
    }
    labels[i / 4] = closest;
    colors[closest].pixels++;
  }
  return { labels, colors };
}

function traceLayer(width, height, labels, index, color, preset) {
  const mask = new Bitmap(width, height);
  for (let i = 0; i < labels.length; i++) mask.data[i] = labels[i] === index ? 0 : 255;
  const tracer = new Potrace({
    threshold: 128, blackOnWhite: true, turdSize: preset.turdSize,
    optTolerance: preset.optTolerance, turnPolicy: Potrace.POLICY_MINORITY, color, background: 'transparent',
  });
  // Potrace receives the same luminance mask as its bundled Node image loader.
  tracer._luminanceData = mask;
  tracer._imageLoaded = true;
  const path = tracer.getPathTag();
  return path.includes('d=""') ? '' : path;
}

async function vectorize({ bytes, mime, preset: requestedPreset, outputFormat, backgroundMode, computeMode, title }) {
  const key = Object.hasOwn(presets, requestedPreset) ? requestedPreset : 'color';
  const preset = presets[key];
  let bitmap = await createImageBitmap(new Blob([bytes], { type: mime }));
  const scale = Math.min(1, preset.maxSide / Math.max(bitmap.width, bitmap.height));
  const width = Math.max(1, Math.round(bitmap.width * scale)), height = Math.max(1, Math.round(bitmap.height * scale));
  const canvas = new OffscreenCanvas(width, height);
  const context = canvas.getContext('2d', { willReadFrequently: true });
  context.drawImage(bitmap, 0, 0, width, height);
  bitmap.close();
  let gpuUsed = false, gpuFallback = false;
  if (computeMode !== 'cpu') {
    try {
      progress(4, 'GPU menyiapkan warna…');
      const blob = await canvas.convertToBlob();
      const base64 = await new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(reader.result.split(',')[1]);
        reader.onerror = reject;
        reader.readAsDataURL(blob);
      });
      const prepared = await gpuPosterize(base64, preset.gpuLevels, preset.maxSide);
      bitmap = await createImageBitmap(new Blob([Uint8Array.from(atob(prepared.base64), char => char.charCodeAt(0))], { type: 'image/png' }));
      context.clearRect(0, 0, width, height);
      context.drawImage(bitmap, 0, 0, width, height);
      bitmap.close();
      gpuUsed = true;
    } catch (error) {
      if (computeMode === 'gpu') throw Error('GPU Vectorizer tidak tersedia/gagal: ' + error.message);
      gpuFallback = true;
    }
  }
  progress(20, 'Memisahkan layer warna…');
  const { labels, colors } = colorLayers(context.getImageData(0, 0, width, height).data, preset.colors);
  canvas.width = canvas.height = 1;
  const palette = colors.map((color, index) => ({ ...color, index }))
    .filter(color => color.pixels >= preset.minPixels).sort((a,b) => b.pixels-a.pixels);
  const layers = [];
  for (const [i, color] of palette.entries()) {
    progress(24 + Math.round((i+1) / palette.length * 62), `Tracing warna ${i+1}/${palette.length} · ${color.color}`);
    const path = traceLayer(width, height, labels, color.index, color.color, preset);
    if (path) layers.push({ color, path });
  }
  if (!layers.length) throw Error('Tidak ada path vector yang berhasil dibuat');
  const groups = layers.map(({ color, path }) => `<g data-color="${color.color}" data-pixels="${color.pixels}">${path}</g>`);
  const svg = `<?xml version="1.0" encoding="UTF-8"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" shape-rendering="geometricPrecision">\n<title>${xml(title)} · ChatX Color Vector</title>\n${groups.join('\n')}\n</svg>\n`;
  const format = ['png', 'webp', 'jpg'].includes(outputFormat) ? outputFormat : 'svg';
  let outWidth = width, outHeight = height, raster = null;
  if (format !== 'svg') {
    progress(96, 'Mengubah output ke ' + format.toUpperCase() + '…');
    outWidth = Math.round(width * 240 / 72);
    outHeight = Math.round(height * 240 / 72);
    const rasterCanvas = new OffscreenCanvas(outWidth, outHeight);
    const ctx = rasterCanvas.getContext('2d');
    if (format !== 'png' || backgroundMode === 'white') {
      ctx.fillStyle = 'white';
      ctx.fillRect(0, 0, outWidth, outHeight);
    }
    ctx.scale(outWidth / width, outHeight / height);
    for (const { color, path } of layers) {
      const geometry = path.match(/<path d="([^"]+)"/)?.[1];
      if (!geometry) throw Error('Geometri hasil Potrace tidak valid.');
      ctx.fillStyle = color.color;
      ctx.fill(new Path2D(geometry), 'evenodd');
    }
    const type = 'image/' + (format === 'jpg' ? 'jpeg' : format);
    const blob = await rasterCanvas.convertToBlob({ type, quality: 0.96 });
    if (blob.type !== type) throw Error('Format keluaran tidak didukung WebView.');
    raster = await blob.arrayBuffer();
  }
  return { svg, raster, outputFormat: format, width: outWidth, height: outHeight,
    preset: key, presetName: preset.name, colorLayers: layers.length,
    palette: palette.map(({ color, pixels }) => ({ color, pixels })),
    backend: gpuUsed ? 'GPU · WebGL2 color prep + CPU · Potrace color layers' : 'CPU · Potrace color layers',
    gpuUsed, gpuFallback };
}

onmessage = async ({ data }) => {
  try {
    const result = await vectorize(data);
    postMessage({ result }, result.raster ? [result.raster] : []);
  } catch (error) {
    postMessage({ error: error.message || String(error) });
  }
};
