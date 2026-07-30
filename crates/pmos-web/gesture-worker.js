// The gesture worker (Architecture §3): MediaPipe HandLandmarker inference,
// GPU-delegated, off the main thread. Receives ImageBitmaps, posts flat
// landmark arrays (hands × 21 × [x,y,z], normalized camera space).
const VISION = "https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@0.10.14";
const MODEL =
  "https://storage.googleapis.com/mediapipe-models/hand_landmarker/hand_landmarker/float16/1/hand_landmarker.task";

let landmarker = null;
let fileset = null;
let makeLandmarker = null;

async function build(opts = {}) {
  landmarker = await makeLandmarker(fileset, {
    baseOptions: { modelAssetPath: MODEL, delegate: "GPU" },
    runningMode: "VIDEO",
    numHands: opts.numHands ?? 2,
    minHandDetectionConfidence: opts.detConf ?? 0.5,
    minTrackingConfidence: opts.trackConf ?? 0.5,
  });
}

(async () => {
  try {
    const { FilesetResolver, HandLandmarker } = await import(`${VISION}/vision_bundle.mjs`);
    fileset = await FilesetResolver.forVisionTasks(`${VISION}/wasm`);
    makeLandmarker = (f, o) => HandLandmarker.createFromOptions(f, o);
    await build();
    postMessage({ type: "ready" });
  } catch (e) {
    postMessage({ type: "error", message: String(e) });
  }
})();

onmessage = async (e) => {
  const m = e.data;
  if (m.type === "configure" && fileset) {
    const old = landmarker;
    landmarker = null;
    old?.close?.();
    await build(m);
    return;
  }
  if (m.type !== "frame" || !landmarker) {
    m.bitmap?.close?.();
    return;
  }
  let result;
  try {
    result = landmarker.detectForVideo(m.bitmap, m.ts);
  } finally {
    m.bitmap.close();
  }
  const hands = result.landmarks.length;
  const data = new Float32Array(hands * 63);
  result.landmarks.forEach((hand, h) =>
    hand.forEach((p, i) => {
      const o = (h * 21 + i) * 3;
      data[o] = p.x;
      data[o + 1] = p.y;
      data[o + 2] = p.z;
    })
  );
  postMessage({ type: "hands", hands, data }, [data.buffer]);
};
