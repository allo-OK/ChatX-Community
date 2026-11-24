const resolution = 1024;
const root = new URL('./onnx/', self.location.href);
let activeSession;

const progress = (stage, pct, text) => postMessage({ progress: { stage, pct, text } });

async function readModel(name) {
  const manifest = await fetch(new URL('models/resources.json', root));
  if (!manifest.ok) throw Error('Manifest model AI tidak tersedia.');
  const model = (await manifest.json())['/models/' + name];
  if (!model) throw Error('Model AI tidak tersedia di build ini.');
  const bytes = new Uint8Array(model.size);
  for (const chunk of model.chunks) {
    const response = await fetch(new URL('models/' + chunk.hash, root));
    if (!response.ok) throw Error('Bagian model AI tidak dapat dibaca.');
    const block = new Uint8Array(await response.arrayBuffer());
    const [start, end] = chunk.offsets;
    if (block.length !== end - start) throw Error('Ukuran bagian model AI tidak cocok.');
    bytes.set(block, start);
    progress('model', 10 + Math.round(35 * end / model.size), 'Memuat model lokal · ' + name);
  }
  return bytes;
}

async function sessionFor(model, gpu) {
  const key = model + '/' + gpu;
  if (activeSession?.key === key) return activeSession;
  if (activeSession) {
    await activeSession.session.release();
    activeSession = null;
  }
  if (gpu && !navigator.gpu) throw Error('WebGPU tidak tersedia pada perangkat ini.');
  const module = gpu ? await import('./onnx/ort.webgpu.bundle.min.mjs') : await import('./onnx/ort.bundle.min.mjs');
  const ort = module.default;
  ort.env.wasm.numThreads = 1;
  ort.env.wasm.proxy = false;
  const wasm = 'ort-wasm-simd-threaded' + (gpu ? '.jsep' : '');
  ort.env.wasm.wasmPaths = { mjs: new URL(wasm + '.mjs', root).href, wasm: new URL(wasm + '.wasm', root).href };
  const bytes = await readModel(model);
  const session = await ort.InferenceSession.create(bytes, {
    executionProviders: [gpu ? 'webgpu' : 'wasm'], graphOptimizationLevel: 'all',
  });
  return activeSession = { key, session, ort };
}

async function decode(bytes, mime) {
  let image;
  try {
    image = await createImageBitmap(new Blob([bytes], { type: mime }));
    const { width, height } = image;
    if (width * height > 64000000 || width > 32767 || height > 32767) throw Error('Dimensi gambar terlalu besar untuk AI.');
    const canvas = new OffscreenCanvas(width, height);
    const context = canvas.getContext('2d', { willReadFrequently: true });
    if (!context) throw Error('Canvas AI tidak dapat dibuat.');
    context.drawImage(image, 0, 0);
    return { canvas, context, width, height };
  } finally {
    image?.close();
  }
}

function bilinear(data, width, height, channels, x, y, channel) {
  const x1 = Math.floor(x), y1 = Math.floor(y);
  const x2 = Math.min(width - 1, Math.ceil(x)), y2 = Math.min(height - 1, Math.ceil(y));
  const dx = x - x1, dy = y - y1;
  const top = (1 - dx) * data[(y1 * width + x1) * channels + channel] + dx * data[(y1 * width + x2) * channels + channel];
  const bottom = (1 - dx) * data[(y2 * width + x1) * channels + channel] + dx * data[(y2 * width + x2) * channels + channel];
  return Math.round((1 - dy) * top + dy * bottom);
}

function inputTensor(ort, pixels, width, height) {
  const stride = resolution * resolution, data = new Float32Array(3 * stride);
  for (let y = 0; y < resolution; y++) {
    for (let x = 0; x < resolution; x++) {
      const index = y * resolution + x;
      for (let channel = 0; channel < 3; channel++) {
        const sample = bilinear(pixels, width, height, 4, x * width / resolution, y * height / resolution, channel);
        data[channel * stride + index] = (sample - 128) / 256;
      }
    }
  }
  return new ort.Tensor('float32', data, [1, 3, resolution, resolution]);
}

function applyMask(pixels, width, height, prediction, model) {
  if (prediction.length !== resolution * resolution) throw Error('Ukuran mask AI tidak dikenali.');
  const mask = Uint8Array.from(prediction, value => Math.max(0, Math.min(255, value * 255)));
  const strength = model === 'small' ? 1.36 : 1.30;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const index = (y * width + x) * 4 + 3;
      const alpha = bilinear(mask, resolution, resolution, 1, x * resolution / width, y * resolution / height, 0);
      const adjusted = alpha <= 5 ? 0 : alpha >= 250 ? 255 : Math.round(255 * Math.pow(alpha / 255, strength));
      pixels[index] = Math.min(pixels[index], adjusted < 7 ? 0 : adjusted);
    }
  }
}

async function removeBackground(payload, surface) {
  const requestedModel = ['small', 'medium', 'large'].includes(payload.model) ? payload.model : 'large';
  const model = requestedModel === 'large' ? 'medium' : requestedModel;
  const { canvas, context, width, height } = surface;
  const rgba = context.getImageData(0, 0, width, height);
  const started = performance.now();
  let useGpu = payload.computeMode !== 'cpu', backend;
  const infer = async gpu => {
    const { session, ort } = await sessionFor(model, gpu);
    progress('inference', 52, gpu ? 'Menghapus background · WebGPU…' : 'Menghapus background · CPU…');
    const input = inputTensor(ort, rgba.data, width, height);
    try {
      const output = await session.run({ input });
      try {
        const prediction = output.output;
        if (!prediction) throw Error('Model AI tidak mengembalikan mask.');
        return new Float32Array(await prediction.getData());
      } finally {
        for (const tensor of Object.values(output)) tensor.dispose();
      }
    } finally {
      input.dispose();
    }
  };
  let prediction;
  try {
    prediction = await infer(useGpu);
  } catch (error) {
    if (!useGpu || payload.computeMode === 'gpu') throw error;
    useGpu = false;
    progress('fallback', 45, 'WebGPU tidak tersedia, lanjut CPU…');
    prediction = await infer(false);
  }
  backend = useGpu ? 'GPU · WebGPU / ONNX Runtime' : 'CPU · ONNX Runtime WASM';
  progress('matting', 92, 'Merapikan tepi subjek…');
  applyMask(rgba.data, width, height, prediction, model);
  context.putImageData(rgba, 0, 0);
  const bytes = await (await canvas.convertToBlob({ type: 'image/png' })).arrayBuffer();
  progress('done', 100, 'Selesai · ' + (useGpu ? 'GPU' : 'CPU'));
  return { bytes, model, requestedModel, backend, elapsedMs: Math.round(performance.now() - started),
    fallbackUsed: requestedModel !== model || (!useGpu && payload.computeMode !== 'cpu') };
}

onmessage = async ({ data }) => {
  let surface;
  try {
    surface = await decode(data.bytes, data.mime);
    const result = data.preview
      ? { bytes: await (await surface.canvas.convertToBlob({ type: 'image/png' })).arrayBuffer() }
      : await removeBackground(data, surface);
    postMessage({ result }, [result.bytes]);
  } catch (error) {
    postMessage({ error: error.message || String(error) });
  } finally {
    if (surface) surface.canvas.width = surface.canvas.height = 1;
  }
};
