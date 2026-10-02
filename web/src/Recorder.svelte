<script lang="ts">
  import { onDestroy } from "svelte";
  import Icon from "./lib/Icon.svelte";
  import { LiveSession, type LiveHandlers } from "./lib/live";
  import { bloom } from "./lib/ui.svelte";

  /**
   * A small recording bar that floats over the note instead of covering it, so a live transcript can
   * be read as it's written. `live` streams the audio for that; without it the recording is only saved.
   */
  let {
    live = null,
    bottom = 24,
    onsave,
    onclose,
  }: { live?: LiveHandlers | null; bottom?: number; onsave: (audio: File) => void; onclose: () => void } = $props();

  let phase = $state<"starting" | "recording" | "stopping" | "denied" | "unsupported">("starting");
  let seconds = $state(0);
  /** Recent input levels, 0-1, drawn as bars. */
  let levels = $state<number[]>(Array(16).fill(0));
  /** Whether words are being sent for a live transcript: on, off after a failure, or not asked for. */
  let liveState = $state<"connecting" | "on" | "off" | null>(null);

  let recorder: MediaRecorder | null = null;
  let stream: MediaStream | null = null;
  let audioCtx: AudioContext | null = null;
  let session: LiveSession | null = null;
  let chunks: Blob[] = [];
  let timer = 0;
  let frame = 0;
  let saving = false;

  /** Opus in WebM where supported (Chrome, Firefox), AAC in MP4 on Safari. */
  function pickType() {
    for (const t of ["audio/webm;codecs=opus", "audio/mp4", "audio/webm", "audio/ogg;codecs=opus"]) {
      if (MediaRecorder.isTypeSupported(t)) return t;
    }
    return "";
  }

  async function start() {
    if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") {
      phase = "unsupported";
      return;
    }
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
    } catch {
      phase = "denied";
      return;
    }
    const type = pickType();
    recorder = new MediaRecorder(stream, type ? { mimeType: type } : undefined);
    recorder.ondataavailable = (e) => e.data.size && chunks.push(e.data);
    recorder.onstop = finish;
    recorder.start(1000);
    phase = "recording";
    const began = performance.now();
    timer = window.setInterval(() => (seconds = Math.floor((performance.now() - began) / 1000)), 250);

    audioCtx = new AudioContext();
    const source = audioCtx.createMediaStreamSource(stream);
    const analyser = audioCtx.createAnalyser();
    analyser.fftSize = 512;
    source.connect(analyser);
    const buf = new Uint8Array(analyser.fftSize);
    let last = 0;
    const tick = (t: number) => {
      frame = requestAnimationFrame(tick);
      if (t - last < 60) return;
      last = t;
      analyser.getByteTimeDomainData(buf);
      let peak = 0;
      for (const v of buf) peak = Math.max(peak, Math.abs(v - 128) / 128);
      levels = [...levels.slice(1), Math.min(1, peak * 2.2)];
    };
    frame = requestAnimationFrame(tick);

    if (live) {
      liveState = "connecting";
      const handlers = live;
      try {
        session = await LiveSession.start(audioCtx, source, {
          ondelta: (text) => ((liveState = "on"), handlers.ondelta(text)),
          onfinal: (text) => ((liveState = "on"), handlers.onfinal(text)),
          onerror: (message) => ((liveState = "off"), handlers.onerror(message)),
        });
        liveState = "on";
      } catch {
        liveState = "off";
        handlers.onerror("This browser can't stream audio for a live transcript");
      }
    }
  }

  async function finish() {
    const blob = new Blob(chunks, { type: recorder?.mimeType || "audio/webm" });
    // The last words arrive after the microphone stops; wait for them before saving.
    if (saving) await session?.stop();
    cleanup();
    if (!saving || blob.size === 0) return onclose();
    const ext = blob.type.includes("mp4") ? "m4a" : blob.type.includes("ogg") ? "ogg" : "webm";
    const stamp = new Date().toISOString().slice(0, 16).replace("T", " ").replace(":", ".");
    onsave(new File([blob], `Voice memo ${stamp}.${ext}`, { type: blob.type.split(";")[0] }));
  }

  function stop(save: boolean) {
    saving = save;
    if (save) phase = "stopping";
    if (recorder && recorder.state !== "inactive") recorder.stop();
    else onclose();
  }

  function cleanup() {
    clearInterval(timer);
    cancelAnimationFrame(frame);
    session?.close();
    stream?.getTracks().forEach((t) => t.stop());
    void audioCtx?.close();
    session = null;
    stream = null;
    audioCtx = null;
  }

  start();
  onDestroy(() => {
    saving = false;
    if (recorder && recorder.state !== "inactive") recorder.stop();
    cleanup();
  });

  const clock = $derived(`${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`);
</script>

<div class="recorder" style:bottom="{bottom}px" role="group" aria-label="Voice memo" transition:bloom>
  {#if phase === "denied" || phase === "unsupported"}
    <span class="message">
      {phase === "denied"
        ? "Gnotes needs microphone access. Allow it in your browser's site settings."
        : "This browser can't record audio here. Recording needs HTTPS."}
    </span>
    <button class="flat icon circular" aria-label="Close" onclick={onclose}><Icon name="close" /></button>
  {:else}
    <span class="clock" class:live={phase === "recording"}><span class="dot"></span>{clock}</span>
    <span class="levels" aria-hidden="true">
      {#each levels as l, i (i)}
        <span style:transform="scaleY({0.12 + l * 0.88})"></span>
      {/each}
    </span>
    {#if liveState}
      <span class="tag" class:on={liveState === "on"} title={liveState === "off" ? "Live transcript stopped; it will be transcribed when saved" : "Words appear in the note as you speak"}>
        {liveState === "off" ? "Not live" : "Live"}
      </span>
    {/if}
    <button class="flat icon circular" aria-label="Discard recording" title="Discard" disabled={phase === "stopping"} onclick={() => stop(false)}>
      <Icon name="close" />
    </button>
    <button class="stop" disabled={phase !== "recording"} aria-label="Stop and add to note" title="Stop and add to note" onclick={() => stop(true)}>
      {#if phase === "stopping"}<span class="spinner"></span>{:else}<span class="square"></span>{/if}
    </button>
  {/if}
</div>

<style>
  .recorder {
    position: absolute;
    left: 50%;
    z-index: 6;
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: calc(100% - 24px);
    padding: 6px 6px 6px 16px;
    translate: -50% 0;
    border-radius: var(--radius-pill);
    background: var(--popover-bg);
    box-shadow: var(--shadow-lg);
  }

  .clock {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--dim-fg);
  }

  .live .dot {
    background: var(--destructive-bg);
    animation: blink 1.2s ease-in-out infinite;
  }

  @keyframes blink {
    50% {
      opacity: 0.25;
    }
  }

  .levels {
    display: flex;
    align-items: center;
    gap: 2px;
    width: 72px;
    height: 22px;
  }

  .levels span {
    flex: 1;
    height: 100%;
    border-radius: var(--radius-pill);
    background: var(--accent-bg);
    transform-origin: center;
    transition: transform 80ms linear;
  }

  .tag {
    padding: 2px 8px;
    border-radius: var(--radius-pill);
    font-size: var(--text-xs);
    font-weight: 700;
    color: var(--dim-fg);
    background: var(--button-bg);
  }

  .tag.on {
    color: var(--accent);
    background: var(--accent-soft);
  }

  .stop {
    flex: none;
    width: 40px;
    height: 40px;
    min-height: 40px;
    padding: 0;
    border-radius: 50%;
    background: var(--destructive-bg);
  }

  .stop:active:not(:disabled) {
    background: var(--destructive-bg);
    transform: scale(0.93);
  }

  .square {
    width: 14px;
    height: 14px;
    border-radius: 3px;
    background: #fff;
  }

  .stop .spinner {
    color: #fff;
  }

  .message {
    font-size: var(--text-sm);
    line-height: 1.35;
  }
</style>
