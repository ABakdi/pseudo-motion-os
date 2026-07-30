// Hand-tracking capture pipeline (Architecture §3, Hand Gestures §2).
// getUserMedia is unavailable inside workers, so video capture lives here on
// the main thread; frames are transferred as ImageBitmaps to the gesture
// worker for MediaPipe inference. Only landmarks ever reach the kernel.
(() => {
  let worker = null;
  let video = null;
  let busy = false;
  let running = false;

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
    }
    // Paced by the camera, not rAF: fires once per new video frame (~30 fps).
    video.requestVideoFrameCallback(() => pump());
  }

  window.pmosGestures = { start };
})();
