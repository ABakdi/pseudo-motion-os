// speech.js — voice capture for PMOS (AI System §5).
//
// DEFAULT ENGINE: Whisper in-browser (whisper-worker.js, transformers.js) —
// works in ANY browser, fully offline after a one-time cached model download,
// audio never leaves the machine. The Web Speech API is kept only as a
// fallback when the Whisper worker cannot initialize (e.g. CDN unreachable
// on first ever run), because it needs a Google/Apple speech backend that
// non-branded Chromium builds (Brave, distro Chromium) don't ship.
//
// Only TEXT crosses into the kernel — mirroring the camera landmarks-only
// privacy boundary.
//
// Driven by kernel voice directives: window.pmosVoice.start() / .stop().
// One utterance per session: 🤙-hold → listen → speak → pause → it executes.
(function () {
  const log = (...a) => console.log("[pmos-voice]", ...a);

  function status(listening, available, reason) {
    window.wasmBindings?.pmos_voice_status(listening, available, reason || "");
  }
  function transcript(text, isFinal) {
    if (text) window.wasmBindings?.pmos_voice_transcript(text, isFinal);
  }

  // ---------- endpointing (energy-based voice activity detection) ----------
  const SPEAK_RMS = 0.015; // above → speech
  const SILENCE_RMS = 0.008; // below → silence (hysteresis)
  const SILENCE_END_S = 1.1; // this much silence after speech ends the utterance
  const NO_SPEECH_S = 6.0; // nothing said at all → give up
  const MAX_UTTERANCE_S = 15.0;
  const INTERIM_EVERY_S = 1.5; // live-transcribe cadence while speaking

  // ---------- whisper engine ----------
  let worker = null;
  let workerReady = false;
  let workerFailed = false;
  let whisperSize = "tiny"; // Settings → Voice; applies to the next session
  let device = "";
  let busy = false;
  let pendingJob = null; // {audio, rate, final} queued while busy/loading

  // mic session state
  let media = null;
  let audioCtx = null;
  let captureRate = 48000; // held past stopMic() — transcribe runs after close
  let chunks = [];
  let chunksLen = 0;
  let active = false;
  let discardResults = false; // Esc during final transcription must not execute
  let speaking = false;
  let sessionT0 = 0;
  let lastVoiceT = 0;
  let lastInterimT = 0;

  const nowS = () => performance.now() / 1000;

  function ensureWorker() {
    if (worker || workerFailed) return;
    worker = new Worker("whisper-worker.js", { type: "module" });
    worker.onmessage = (e) => {
      const m = e.data;
      if (m.type === "progress") {
        if (active) status(true, true, `downloading speech model… ${m.pct}%`);
      } else if (m.type === "ready") {
        workerReady = true;
        device = m.device;
        log("whisper ready on", device);
        if (active) status(true, true, "");
        flushJob();
      } else if (m.type === "result") {
        busy = false;
        log("result:", m.final ? `final "${m.text}"` : `interim "${m.text}"`);
        if (discardResults) {
          pendingJob = null;
        } else if (m.final) {
          if (m.text) transcript(m.text, true);
          // Continuous: capture is still rolling — only report stopped
          // when the session actually ended.
          if (!active) status(false, true, m.text ? "" : "no speech heard");
        } else if (active && m.text) {
          transcript(m.text, false);
        }
        flushJob();
      } else if (m.type === "error") {
        log("whisper error:", m.error);
        busy = false;
        workerFailed = true;
        worker.terminate();
        worker = null;
        if (active) {
          stopMic();
          active = false;
          // Retry the whole session on the fallback engine.
          startWebSpeech();
        }
      }
    };
    worker.onerror = (e) => {
      log("worker failed to load:", e.message || e);
      workerFailed = true;
      worker = null;
      if (active) {
        stopMic();
        active = false;
        startWebSpeech();
      }
    };
    worker.postMessage({
      type: "init",
      lang: navigator.language || "en-US",
      size: whisperSize,
    });
  }

  function flushJob() {
    if (pendingJob && workerReady && !busy) {
      const j = pendingJob;
      pendingJob = null;
      busy = true;
      worker.postMessage({ type: "transcribe", ...j }, [j.audio.buffer]);
    }
  }

  function snapshot() {
    const all = new Float32Array(chunksLen);
    let o = 0;
    for (const c of chunks) {
      all.set(c, o);
      o += c.length;
    }
    return all;
  }

  function transcribe(final) {
    if (!chunksLen) return;
    pendingJob = { audio: snapshot(), rate: captureRate, final };
    flushJob();
  }

  function stopMic() {
    media?.getTracks().forEach((t) => t.stop());
    media = null;
    audioCtx?.close().catch(() => {});
    audioCtx = null;
  }

  function endUtterance(gotSpeech) {
    // CONTINUOUS mode (Voice Kit): an utterance ending does NOT stop the
    // mic — the buffer flushes to the transcriber and capture keeps rolling
    // until stop() (the RECORD sign / widget click).
    if (gotSpeech) {
      log("utterance ended, transcribing", (chunksLen / captureRate).toFixed(1) + "s of audio");
      transcribe(true);
    }
    chunks = [];
    chunksLen = 0;
    speaking = false;
    sessionT0 = nowS();
  }

  function onFrame(frame) {
    if (!active) return;
    chunks.push(frame);
    chunksLen += frame.length;
    let sum = 0;
    for (let i = 0; i < frame.length; i++) sum += frame[i] * frame[i];
    const rms = Math.sqrt(sum / frame.length);
    const t = nowS();
    if (rms > SPEAK_RMS) {
      if (!speaking) {
        speaking = true;
        lastInterimT = t;
        log("speech started");
      }
      lastVoiceT = t;
    }
    if (!speaking) {
      if (t - sessionT0 > NO_SPEECH_S) {
        // Nothing said: drop the silence buffer, keep listening.
        chunks = [];
        chunksLen = 0;
        sessionT0 = t;
      }
      return;
    }
    if (rms < SILENCE_RMS && t - lastVoiceT > SILENCE_END_S) {
      endUtterance(true);
    } else if (t - sessionT0 > MAX_UTTERANCE_S) {
      endUtterance(true);
    } else if (t - lastInterimT > INTERIM_EVERY_S && workerReady && !busy) {
      lastInterimT = t;
      transcribe(false);
    }
  }

  const WORKLET_SRC = `
    class PmosCapture extends AudioWorkletProcessor {
      constructor() { super(); this.buf = []; this.len = 0; }
      process(inputs) {
        const ch = inputs[0][0];
        if (ch) {
          this.buf.push(ch.slice(0));
          this.len += ch.length;
          if (this.len >= 2048) {
            const all = new Float32Array(this.len);
            let o = 0;
            for (const c of this.buf) { all.set(c, o); o += c.length; }
            this.port.postMessage(all, [all.buffer]);
            this.buf = []; this.len = 0;
          }
        }
        return true;
      }
    }
    registerProcessor('pmos-capture', PmosCapture);
  `;

  async function startWhisper() {
    ensureWorker();
    if (workerFailed) {
      startWebSpeech();
      return;
    }
    try {
      media = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, noiseSuppression: true },
      });
      audioCtx = new AudioContext();
      captureRate = audioCtx.sampleRate;
      const url = URL.createObjectURL(new Blob([WORKLET_SRC], { type: "application/javascript" }));
      await audioCtx.audioWorklet.addModule(url);
      URL.revokeObjectURL(url);
      const node = new AudioWorkletNode(audioCtx, "pmos-capture");
      node.port.onmessage = (e) => onFrame(e.data);
      audioCtx.createMediaStreamSource(media).connect(node);

      chunks = [];
      chunksLen = 0;
      active = true;
      discardResults = false;
      speaking = false;
      pendingJob = null;
      sessionT0 = nowS();
      log("mic capturing at", audioCtx.sampleRate, "Hz; whisper", workerReady ? "warm" : "loading");
      status(true, true, workerReady ? "" : "loading speech model…");
    } catch (err) {
      log("mic failed:", err);
      stopMic();
      status(false, false, "microphone unavailable: " + (err.name || err));
    }
  }

  // ---------- Web Speech fallback (needs a browser speech backend) ----------
  let rec = null;
  let recCancelled = false;

  function startWebSpeech() {
    const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
    if (!SR) {
      status(false, false, "no speech engine available (Whisper failed and this browser has no Web Speech backend)");
      return;
    }
    if (rec) return;
    log("using Web Speech fallback");
    recCancelled = false;
    let errored = false;
    rec = new SR();
    rec.lang = navigator.language || "en-US";
    rec.continuous = false;
    rec.interimResults = true;
    rec.onstart = () => status(true, true, "");
    rec.onresult = (e) => {
      let interim = "";
      let final_ = "";
      for (let i = e.resultIndex; i < e.results.length; i++) {
        const r = e.results[i];
        if (r.isFinal) final_ += r[0].transcript;
        else interim += r[0].transcript;
      }
      if (final_) transcript(final_.trim(), true);
      else transcript(interim.trim(), false);
    };
    rec.onerror = (e) => {
      log("web speech error:", e.error);
      if (recCancelled) return;
      errored = true;
      const fatal = ["not-allowed", "service-not-allowed", "audio-capture"].includes(e.error);
      status(false, !fatal, e.error === "no-speech" ? "no speech heard" : "speech error: " + e.error);
    };
    rec.onend = () => {
      rec = null;
      if (!recCancelled && !errored) status(false, true, "");
    };
    try {
      rec.start();
    } catch (err) {
      rec = null;
      status(false, false, String(err));
    }
  }

  // ---------- public interface (driven by kernel directives) ----------
  window.pmosVoice = {
    /// Settings → Voice: swap the Whisper model size; a changed size
    /// tears down the warm worker so the next session loads the new model.
    configure(opts) {
      const size = opts?.whisper;
      if (size && size !== whisperSize) {
        log("whisper size →", size);
        whisperSize = size;
        if (worker) {
          worker.terminate();
          worker = null;
        }
        workerReady = false;
        workerFailed = false;
        busy = false;
        pendingJob = null;
      }
    },
    start() {
      if (active || rec) return;
      startWhisper();
    },
    stop() {
      discardResults = true;
      pendingJob = null;
      if (active) {
        active = false;
        stopMic();
      }
      if (rec) {
        recCancelled = true;
        try {
          rec.abort();
        } catch (_) {}
        rec = null;
      }
      status(false, true, "");
    },
  };
})();
