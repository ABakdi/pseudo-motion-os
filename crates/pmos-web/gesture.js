// Hand-tracking capture pipeline (Architecture §3, Hand Gestures §2).
// getUserMedia is unavailable inside workers, so video capture lives here on
// the main thread; frames are transferred as ImageBitmaps to the gesture
// worker for MediaPipe inference. Only landmarks enter the kernel; preview
// pixels (when the Hand Tracker viewer requests them) go straight to the
// shell overlay and are never stored.
(() => {
  const PREVIEW_W = 320;
  const PREVIEW_H = 240;
  let worker = null;
  let video = null;
  let busy = false;
  let running = false;
  let streamFeed = false;
  let preview = null; // lazy 2D canvas for preview downscale

  async function start() {
    if (running) return;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        video: { width: 640, height: 480, facingMode: "user" },
      });
      video = document.createElement("video");
      video.srcObject = stream;
      video.muted = true;
      video.playsInline = true;
      await video.play();

      worker = new Worker("gesture-worker.js", { type: "module" });
      worker.onmessage = (e) => {
        const m = e.data;
        if (m.type === "ready") {
          running = true;
          window.wasmBindings.pmos_camera_status(true);
          pump();
        } else if (m.type === "hands") {
          busy = false;
          window.wasmBindings.pmos_hands_frame(m.data, m.hands);
        } else if (m.type === "error") {
          console.error("[pmos gestures] worker:", m.message);
          window.wasmBindings.pmos_camera_status(false);
        }
      };
    } catch (err) {
      console.warn("[pmos gestures] camera unavailable:", err);
      window.wasmBindings?.pmos_camera_status(false);
    }
  }

  async function pump() {
    if (!running) return;
    if (!busy && video.readyState >= 2) {
      busy = true;
      const bitmap = await createImageBitmap(video);
      worker.postMessage({ type: "frame", bitmap, ts: performance.now() }, [bitmap]);
      if (streamFeed) sendPreview();
    }
    // Paced by the camera, not rAF: fires once per new video frame (~30 fps).
    video.requestVideoFrameCallback(() => pump());
  }

  // Mirrored, downscaled preview for the Hand Tracker window (selfie view).
  function sendPreview() {
    if (!preview) {
      preview = document.createElement("canvas");
      preview.width = PREVIEW_W;
      preview.height = PREVIEW_H;
    }
    const c = preview.getContext("2d");
    c.save();
    c.scale(-1, 1);
    c.drawImage(video, -PREVIEW_W, 0, PREVIEW_W, PREVIEW_H);
    c.restore();
    const px = c.getImageData(0, 0, PREVIEW_W, PREVIEW_H);
    window.wasmBindings.pmos_camera_frame(new Uint8Array(px.data.buffer), PREVIEW_W, PREVIEW_H);
  }

  // Applied by the platform glue from kernel directives (ABI 1.2).
  function configure(opts) {
    streamFeed = !!opts.streamFeed;
    if (worker && (opts.numHands || opts.detConf || opts.trackConf)) {
      worker.postMessage({
        type: "configure",
        numHands: opts.numHands,
        detConf: opts.detConf,
        trackConf: opts.trackConf,
      });
    }
  }

  window.pmosGestures = { start, configure };
})();
