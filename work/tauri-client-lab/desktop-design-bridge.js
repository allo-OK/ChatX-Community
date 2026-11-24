const kcDesktopDesign = (() => {
  const invoke = (...args) => window.__TAURI__.core.invoke(...args);
  async function detectFacesOnWindows(image) {
    const canvas = document.createElement('canvas');
    const width = image.naturalWidth, height = image.naturalHeight;
    const scale = Math.min(1, 1600 / Math.max(width, height));
    canvas.width = Math.max(1, Math.round(width * scale));
    canvas.height = Math.max(1, Math.round(height * scale));
    canvas.getContext('2d').drawImage(image, 0, 0, canvas.width, canvas.height);
    const xScale = width / canvas.width, yScale = height / canvas.height;
    const png = canvas.toDataURL('image/png');
    canvas.width = canvas.height = 0;
    const faces = await invoke('detect_crop_faces', { png });
    return faces.map(face => ({ boundingBox: new DOMRect(
      face.x * xScale, face.y * yScale, face.width * xScale, face.height * yScale
    ) }));
  }
  if (typeof window.FaceDetector !== 'function') {
    window.FaceDetector = class {
      detect(image) { return detectFacesOnWindows(image); }
    };
  }
  let upscaleListener;
  let vectorListener;
  let backgroundListener, backgroundWorker, backgroundIdle, parallelBackground = false;
  let backgroundQueue = Promise.resolve();
  const base64Blob = blob => new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result.split(',')[1]);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
  async function processBackground(input, preview, parallel) {
    const encoded = String(input.base64 || '').replace(/^data:image\/[^;]+;base64,/, '');
    if (!encoded || encoded.length > 112 * 1024 * 1024) throw Error('Data gambar kosong atau terlalu besar. Maksimal 80 MB.');
    const bytes = Uint8Array.from(atob(encoded), char => char.charCodeAt(0)).buffer;
    if (bytes.byteLength > 80 * 1024 * 1024) throw Error('Gambar terlalu besar. Maksimal 80 MB.');
    const createWorker = () => new Worker(new URL('rust-background-worker.js', location.href), { type: 'module' });
    if (!parallel) clearTimeout(backgroundIdle);
    const worker = parallel ? createWorker() : (backgroundWorker ||= createWorker());
    let timer, failed = true;
    try {
      const { bytes: output, ...details } = await new Promise((resolve, reject) => {
        timer = setTimeout(() => reject(Error('Proses AI melewati batas waktu 4 menit.')), 240000);
        worker.onerror = event => reject(Error(event.message || 'Worker Hapus Background gagal dijalankan.'));
        worker.onmessage = ({ data }) => {
          if (data.progress) backgroundListener?.(data.progress);
          else if (data.error) reject(Error(data.error));
          else if (data.result) resolve(data.result);
        };
        worker.postMessage({ bytes, mime: input.mime || 'image/png', model: input.model, computeMode: input.computeMode, preview }, [bytes]);
      });
      const base64 = await base64Blob(new Blob([output]));
      failed = false;
      return { ...details, base64, mime: 'image/png', ok: true };
    } finally {
      clearTimeout(timer);
      worker.onmessage = worker.onerror = null;
      if (parallel || failed) {
        worker.terminate();
        if (!parallel) backgroundWorker = null;
      } else {
        backgroundIdle = setTimeout(() => {
          worker.terminate();
          if (backgroundWorker === worker) backgroundWorker = null;
        }, 120000);
      }
    }
  }
  async function checkDesignAccess() {
    const policy = await window.kc?.refreshFeaturePolicy?.();
    if (policy?.banned === true || policy?.designAI === false) {
      throw Error('Akses alat desain / AI dibatasi HOST');
    }
  }
  function queueBackground(input, preview = false) {
    const run = parallel => checkDesignAccess().then(() => processBackground(input, preview, parallel))
      .catch(error => ({ ok: false, error: error.message || String(error) }));
    if (input.parallel && !parallelBackground && !preview) {
      parallelBackground = true;
      return run(true).finally(() => { parallelBackground = false; });
    }
    const job = backgroundQueue.then(() => run(false));
    backgroundQueue = job;
    return job;
  }
  async function runVector(input) {
    await checkDesignAccess();
    const source = await invoke('read_local_image', { path: input.input, preview: false });
    if (!source.ok) throw Error(source.error || 'File gambar tidak ditemukan.');
    const bytes = Uint8Array.from(atob(source.base64), char => char.charCodeAt(0)).buffer;
    const title = source.name.replace(/\.[^.]+$/, '').slice(0, 120);
    const worker = new Worker(new URL('rust-vector-worker.js', location.href), { type: 'module' });
    let timer;
    try {
      const result = await new Promise((resolve, reject) => {
        timer = setTimeout(() => reject(Error('Vectorizer dihentikan otomatis setelah 4 menit.')), 240000);
        worker.onerror = event => reject(Error(event.message || 'Worker Vectorizer gagal dijalankan.'));
        worker.onmessage = ({ data }) => {
          if (data.progress) vectorListener?.(data.progress);
          else if (data.error) reject(Error(data.error));
          else if (data.result) resolve(data.result);
        };
        worker.postMessage({ bytes, mime: source.type, preset: input.preset, title,
          outputFormat: input.outputFormat, backgroundMode: input.backgroundMode, computeMode: input.computeMode }, [bytes]);
      });
      const { raster, svg, ...details } = result;
      const raster64 = raster ? await base64Blob(new Blob([raster])) : null;
      const saved = await invoke('write_vector_output', { input: { input: input.input, outputDir: input.outputDir,
        svg, raster: raster64, preset: details.preset, outputFormat: details.outputFormat } });
      if (!saved.ok) throw Error(saved.error || 'Gagal menyimpan hasil vektor.');
      const format = details.outputFormat;
      const preview = raster64 ? 'data:image/' + (format === 'jpg' ? 'jpeg' : format) + ';base64,' + raster64
        : 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
      return { ...details, ...saved, preview };
    } finally {
      clearTimeout(timer);
      worker.terminate();
    }
  }
  const methods = Object.freeze({
    setNativeTheme: theme => invoke('set_native_theme', {theme}),
    async runHealthCheck(role, checkHost) {
      const checks = [{name:'Role aplikasi', ok:true, detail:role}];
      const probe = async (name, check) => {
        try { return {name, ...await check()}; }
        catch (error) { return {name, ok:false, detail:error.message || String(error)}; }
      };
      const asset = (file, detail) => async () => {
        const response = await fetch(new URL(file, location.href), {signal:AbortSignal.timeout(2500)});
        return {ok:response.ok, detail:response.ok ? detail : 'HTTP '+response.status};
      };
      if (checkHost) checks.push(await probe('Server lokal', checkHost));
      checks.push(...await Promise.all([
        probe('Storage tulis', () => invoke('check_health_storage')),
        probe('Upscale GPU / Vulkan', async () => {
          const result = await invoke('upscale_status');
          return {ok:result.ok, detail:result.ok ? result.engine : result.error};
        }),
        probe('Vectorizer worker', asset('rust-vector-worker.js', 'Worker Potrace tersedia')),
        probe('Remove Background worker', asset('rust-background-worker.js', 'Worker tersedia')),
      ]));
      return {ok:checks.every(check => check.ok), checks};
    },
    reloadUiSafe() {
      setTimeout(() => location.reload(), 0);
      return Promise.resolve({ok:true});
    },
    chooseImageFiles: options => invoke('choose_image_files', { multiple: options?.multiple === true }),
    chooseUpscaleImage: async () => (await invoke('choose_image_files', { multiple: false }))[0] || null,
    chooseUpscaleOutput: () => invoke('choose_upscale_output'),
    chooseVectorOutput: () => invoke('choose_vector_output'),
    vectorizerStatus: async () => ({ ok: true, engine: 'Potrace Worker', presets: ['logo', 'color', 'detail'] }),
    onVectorProgress: callback => { vectorListener = callback; },
    vectorizeImage: async input => {
      try { return await runVector(input); }
      catch (error) { return { ok: false, error: error.message || String(error) }; }
    },
    aiPreviewImage: input => queueBackground(input, true),
    aiRemoveBackground: input => queueBackground(input),
    onAiBgProgress: callback => { backgroundListener = callback; },
    writeBase64File: input => invoke('write_base64_file', { input }),
    savePng: input => invoke('save_png', { input }),
    readLocalImagePreview: path => invoke('read_local_image', { path, preview: true }),
    readFile: path => invoke('read_local_image', { path, preview: false }),
    openPath: path => invoke('open_image_path', { path }),
    upscaleStatus: () => invoke('upscale_status'),
    onUpscaleProgress: callback => { upscaleListener = callback; },
    upscaleImage: async input => {
      const refresh = async () => {
        try { upscaleListener?.(await invoke('upscale_progress')); } catch {}
      };
      const timer = setInterval(refresh, 300);
      try {
        return await invoke('upscale_image', { input });
      } finally {
        clearInterval(timer);
        await refresh();
      }
    },
  });
  return { methods, detectFacesOnWindows, base64Blob };
})();
