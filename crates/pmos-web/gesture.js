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
  let busySince = 0;
  let running = false;
  let streamFeed = false;
  let preview = null; // lazy 2D canvas for preview downscale
  // Worker-side config actually in effect — configure() only rebuilds the
  // landmarker when these change (a rebuild interrupts tracking briefly).
  let workerCfg = "2,0.5,0.5";

  function reasonFor(err) {
    switch (err && err.name) {
      case "NotAllowedError":
        return "camera permission is blocked — click the camera/padlock icon in the address bar, allow camera access, then press Enable again";
      case "NotFoundError":
        return "no camera device was found on this machine";
      case "NotReadableError":
        return "the camera is in use by another application";
      default:
        return "camera error: " + ((err && err.message) || String(err));
    }
  }

  async function start() {
    if (running) return;
    try {
      window.wasmBindings?.pmos_camera_status(false, "requesting camera…");
      const stream = await navigator.mediaDevices.getUserMedia({
        video: { width: 640, height: 480, facingMode: "user" },
      });
      window.wasmBindings?.pmos_camera_status(false, "camera on — loading hand tracking model…");
      video = document.createElement("video");
      video.muted = true;
      video.playsInline = true;
      video.setAttribute("playsinline", "");
      // Must live in the document — some browsers reject play() on detached
      // elements. Invisible, but never display:none (that stalls frames).
      video.style.cssText =
        "position:fixed;right:0;bottom:0;width:2px;height:2px;opacity:0;pointer-events:none;";
      document.body.appendChild(video);
      video.srcObject = stream;
      await video.play();

      worker = new Worker("gesture-worker.js", { type: "module" });
      worker.onmessage = (e) => {
        const m = e.data;
        if (m.type === "ready") {
          running = true;
          if (faceEnabled) worker.postMessage({ type: "face", enable: true });
          window.wasmBindings.pmos_camera_status(true);
          pump();
        } else if (m.type === "hands") {
          busy = false;
          window.wasmBindings.pmos_hands_frame(m.data, m.hands);
        } else if (m.type === "face") {
          window.wasmBindings.pmos_face_frame(m.blinkL, m.blinkR, m.jaw ?? 0);
        } else if (m.type === "error") {
          console.error("[pmos gestures] worker:", m.message);
          window.wasmBindings.pmos_camera_status(
            false,
            "hand-tracking model failed to load (network needed on first run): " + m.message
          );
        }
      };
    } catch (err) {
      console.warn("[pmos gestures] camera unavailable:", err);
      window.wasmBindings?.pmos_camera_status(false, reasonFor(err));
    }
  }

  async function pump() {
    if (!running) return;
    // The preview is independent of the worker — keep it flowing always.
    if (streamFeed && video.readyState >= 2) sendPreview();
    if (!busy && video.readyState >= 2) {
      busy = true;
      busySince = performance.now();
      const bitmap = await createImageBitmap(video);
      worker.postMessage({ type: "frame", bitmap, ts: performance.now() }, [bitmap]);
    } else if (busy && performance.now() - busySince > 2000) {
      // Watchdog: a worker reply was lost (e.g. mid-rebuild) — recover.
      busy = false;
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

  let faceEnabled = false;

  // Applied by the platform glue from kernel directives (ABI 1.2).
  function configure(opts) {
    if (typeof opts.face === "boolean") {
      faceEnabled = opts.face;
      worker?.postMessage({ type: "face", enable: faceEnabled });
      if (opts.streamFeed === undefined) return; // face-only update
    }
    streamFeed = !!opts.streamFeed;
    // Rebuild the landmarker ONLY when worker-side tuning really changed —
    // viewer open/close and feed toggles must never interrupt tracking.
    const key = [opts.numHands ?? 2, opts.detConf ?? 0.5, opts.trackConf ?? 0.5].join(",");
    if (worker && key !== workerCfg) {
      workerCfg = key;
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
