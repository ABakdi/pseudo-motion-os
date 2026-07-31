// speech.js — voice capture via the browser's Web Speech API (AI System §5).
//
// v1 STT engine: free, zero-download, no API key. Chrome/Edge process audio
// via the browser's speech service (documented limitation; Whisper-in-browser
// is the planned v2 behind the same interface). Only TEXT crosses into the
// kernel — raw audio never leaves the browser engine, mirroring the camera
// privacy boundary (landmarks-only).
//
// Known platform gap: non-branded Chromium builds (e.g. distro packages) ship
// WITHOUT Google's speech service keys — the API exists but every session
// fails with a `network` error. Branded Google Chrome / Edge work.
//
// Driven by kernel voice directives: window.pmosVoice.start() / .stop().
// One utterance per session (continuous=false): 🤙-hold → listen → speak →
// the final transcript auto-executes and the session ends itself.
(function () {
  const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
  let rec = null;
  let cancelled = false;
  let errored = false;

  const log = (...a) => console.log("[pmos-voice]", ...a);

  function status(listening, available, reason) {
    window.wasmBindings?.pmos_voice_status(listening, available, reason || "");
  }
  function transcript(text, isFinal) {
    if (text) window.wasmBindings?.pmos_voice_transcript(text, isFinal);
  }

  const REASONS = {
    "not-allowed": "microphone permission denied",
    "service-not-allowed": "the browser's speech service is blocked",
    "audio-capture": "no microphone found",
    "no-speech": "no speech heard",
    network:
      "speech service unreachable — non-branded Chromium has no speech backend; use Google Chrome/Edge (and check network)",
    aborted: "",
  };
  // Errors that mean the engine can't work at all (vs. a one-off miss).
  const FATAL = ["not-allowed", "service-not-allowed", "audio-capture"];

  window.pmosVoice = {
    start() {
      if (!SR) {
        log("SpeechRecognition API not present");
        status(false, false, "speech recognition is unavailable in this browser");
        return;
      }
      if (rec) return; // already listening
      cancelled = false;
      errored = false;
      rec = new SR();
      rec.lang = navigator.language || "en-US";
      rec.continuous = false; // one utterance per activation
      rec.interimResults = true; // live text while speaking
      rec.maxAlternatives = 1;
      rec.onstart = () => {
        log("engine started, lang =", rec?.lang);
        status(true, true, "");
      };
      rec.onaudiostart = () => log("audio capture started");
      rec.onspeechstart = () => log("speech detected");
      rec.onresult = (e) => {
        let interim = "";
        let final_ = "";
        for (let i = e.resultIndex; i < e.results.length; i++) {
          const r = e.results[i];
          if (r.isFinal) final_ += r[0].transcript;
          else interim += r[0].transcript;
        }
        log("result:", final_ ? `final "${final_}"` : `interim "${interim}"`);
        if (final_) transcript(final_.trim(), true);
        else transcript(interim.trim(), false);
      };
      rec.onerror = (e) => {
        log("error:", e.error, e.message || "");
        if (cancelled) return;
        errored = true;
        status(false, !FATAL.includes(e.error), REASONS[e.error] ?? "speech error: " + e.error);
      };
      rec.onend = () => {
        log("engine ended");
        rec = null;
        // If onerror already reported, don't overwrite its reason with a
        // generic clean-end status (both can land in the same frame).
        if (!cancelled && !errored) status(false, true, "");
      };
      try {
        rec.start();
        log("start requested");
      } catch (err) {
        log("start threw:", err);
        rec = null;
        status(false, false, String(err));
      }
    },
    stop() {
      cancelled = true;
      if (rec) {
        try {
          rec.abort();
        } catch (_) {}
        rec = null;
      }
      status(false, true, "");
    },
  };
})();
