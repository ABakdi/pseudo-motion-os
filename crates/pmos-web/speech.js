// speech.js — voice capture via the browser's Web Speech API (AI System §5).
//
// v1 STT engine: free, zero-download, no API key. Chrome/Edge process audio
// via the browser's speech service (documented limitation; Whisper-in-browser
// is the planned v2 behind the same interface). Only TEXT crosses into the
// kernel — raw audio never leaves the browser engine, mirroring the camera
// privacy boundary (landmarks-only).
//
// Driven by kernel voice directives: window.pmosVoice.start() / .stop().
// One utterance per session (continuous=false): 🤙-hold → listen → speak →
// the final transcript auto-executes and the session ends itself.
(function () {
  const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
  let rec = null;
  let cancelled = false;

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
    network: "speech service unreachable (Web Speech needs network in this browser)",
    aborted: "",
  };
  // Errors that mean the engine can't work at all (vs. a one-off miss).
  const FATAL = ["not-allowed", "service-not-allowed", "audio-capture"];

  window.pmosVoice = {
    start() {
      if (!SR) {
        status(false, false, "speech recognition is unavailable in this browser");
        return;
      }
      if (rec) return; // already listening
      cancelled = false;
      rec = new SR();
      rec.lang = navigator.language || "en-US";
      rec.continuous = false; // one utterance per activation
      rec.interimResults = true; // live text while speaking
      rec.maxAlternatives = 1;
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
        if (cancelled) return;
        status(false, !FATAL.includes(e.error), REASONS[e.error] ?? "speech error: " + e.error);
      };
      rec.onend = () => {
        rec = null;
        if (!cancelled) status(false, true, "");
      };
      try {
        rec.start();
      } catch (err) {
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
