// figstudio: runs a shader's module on WebGPU in a worker started by a sandboxed frame: an
// opaque origin (no access to this page or its storage) under the frame's policy, which
// allows no network. The worker has its own thread, so a run that takes too long (an endless
// loop, or a built-in that never returns) cannot block the page, and replacing the frame ends
// the worker with it, in every browser (WebKit runs the frame itself on the page's thread).
// Defines globalThis.__figstudioShaders.run.
(() => {
  // Runs in the worker: one WebGPU device, modules kept by their text.
  const WORKER = `let dev = null;
async function device() {
  if (!dev) dev = (async () => {
    if (!navigator.gpu) throw new Error('no WebGPU');
    const a = await navigator.gpu.requestAdapter();
    if (!a) throw new Error('no WebGPU adapter');
    return a.requestDevice();
  })();
  return dev;
}
const modules = new Map();
onmessage = async (e) => {
  const { id, js, params, width, height, time, input } = e.data;
  try {
    const d = await device();
    let mod = modules.get(js);
    if (!mod) {
      // A data: URL: WebKit refuses a blob: module made in an opaque-origin worker.
      mod = await import('data:text/javascript,' + encodeURIComponent(js));
      modules.set(js, mod);
    }
    const output = d.createTexture({ size: [width, height], format: 'rgba8unorm',
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_SRC });
    let inputTex = null;
    if (input) {
      inputTex = d.createTexture({ size: [width, height], format: 'rgba8unorm',
        usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST | GPUTextureUsage.COPY_SRC });
      d.queue.writeTexture({ texture: inputTex }, new Uint8Array(input), { bytesPerRow: width * 4 }, [width, height]);
    }
    const frame = { params, state: {}, input: inputTex, output, time, deltaTime: 0, frame: 0,
      inputPadding: 0, outputPadding: 0, mousePosition: { x: 0, y: 0 } };
    d.pushErrorScope('validation');
    // A fresh step budget per run (the module counts its loops and calls; see ts.rs).
    for (const k of Object.keys(mod)) if (k.startsWith('__fsr_')) mod[k]();
    if (mod.setup) await mod.setup(d, frame);
    await mod.render(d, frame);
    const row = Math.ceil(width * 4 / 256) * 256;
    const buf = d.createBuffer({ size: row * height, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
    const enc = d.createCommandEncoder();
    enc.copyTextureToBuffer({ texture: output }, { buffer: buf, bytesPerRow: row }, [width, height]);
    d.queue.submit([enc.finish()]);
    const err = await d.popErrorScope();
    if (err) throw new Error(err.message);
    await buf.mapAsync(GPUMapMode.READ);
    const src = new Uint8Array(buf.getMappedRange());
    const out = new Uint8Array(width * height * 4);
    for (let y = 0; y < height; y++) out.set(src.subarray(y * row, y * row + width * 4), y * width * 4);
    buf.unmap(); buf.destroy(); output.destroy(); if (inputTex) inputTex.destroy();
    postMessage({ id, ok: true, pixels: out.buffer }, [out.buffer]);
  } catch (err) {
    postMessage({ id, ok: false, error: String((err && err.message) || err) });
  }
};
postMessage({ ready: true });`;
  // Runs in the frame: starts the worker (which inherits the frame's policy) and passes
  // messages between it and this page.
  const FRAME = `const w = new Worker('data:text/javascript,' + encodeURIComponent(${JSON.stringify(WORKER)}));
w.onmessage = (e) => { const d = e.data; parent.postMessage(d, '*', d && d.pixels ? [d.pixels] : []); };
w.onerror = (e) => parent.postMessage({ fatal: String(e.message || 'the shader worker failed') }, '*');
onmessage = (e) => { if (e.source !== parent) return; const d = e.data; w.postMessage(d, d && d.input ? [d.input] : []); };`;
  // No network, no frames, no storage: its own inline script, the data: worker and modules.
  const CSP = "default-src 'none'; script-src 'unsafe-inline' data:; worker-src data:";
  let frame = null, ready = null, onReady = null, onFail = null, next = 1;
  const waiting = new Map();
  addEventListener('message', (e) => {
    if (!frame || e.source !== frame.contentWindow || !e.data) return;
    if (e.data.ready) { if (onReady) onReady(); return; }
    // The worker did not start (no workers or no WebGPU there): every run fails.
    if (e.data.fatal) { replace(e.data.fatal); return; }
    const w = waiting.get(e.data.id); if (!w) return;
    waiting.delete(e.data.id);
    e.data.ok ? w.resolve(new Uint8Array(e.data.pixels)) : w.reject(new Error(e.data.error));
  });
  function sandbox() {
    if (ready) return ready;
    ready = new Promise((resolve, reject) => {
      onReady = resolve;
      onFail = reject;
      frame = document.createElement('iframe');
      frame.setAttribute('sandbox', 'allow-scripts');
      frame.style.display = 'none';
      frame.srcdoc = `<!doctype html><meta http-equiv="Content-Security-Policy" content="${CSP}"><script>${FRAME}<\/script>`;
      document.body.appendChild(frame);
    });
    return ready;
  }
  // Removes the frame (ending its worker); waiting runs fail with `why`.
  function replace(why) {
    if (frame) frame.remove();
    if (onFail) onFail(new Error(why || 'the shader took too long'));
    frame = null; ready = null; onReady = null; onFail = null;
    for (const [, w] of waiting) w.reject(new Error(why || 'the shader took too long'));
    waiting.clear();
  }
  const runs = { ok: 0, failed: 0, lastError: null };
  globalThis.__figstudioShaders = {
    // Counts for diagnostics and the browser test.
    runs,
    async run(js, params, width, height, time, input, timeout) {
      try {
        const pixels = await start(js, params, width, height, time, input, timeout);
        runs.ok++;
        return pixels;
      } catch (e) {
        runs.failed++;
        runs.lastError = String(e && e.message || e);
        throw e;
      }
    },
  };
  async function start(js, params, width, height, time, input, timeout) {
      await sandbox();
      const id = next++;
      return new Promise((resolve, reject) => {
        const buf = input ? input.slice().buffer : null;
        // Too long (an endless loop?): the sandbox is replaced; waiting runs fail.
        const timer = setTimeout(() => replace(), timeout);
        waiting.set(id, { resolve: (v) => { clearTimeout(timer); resolve(v); }, reject: (e) => { clearTimeout(timer); reject(e); } });
        frame.contentWindow.postMessage({ id, js, params, width, height, time, input: buf }, '*', buf ? [buf] : []);
      });
  }
})();
