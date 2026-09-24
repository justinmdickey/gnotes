<script lang="ts">
  import { onDestroy } from "svelte";
  import Dialog from "./lib/Dialog.svelte";
  import Icon from "./lib/Icon.svelte";

  let { onsave, onclose }: { onsave: (audio: File) => void; onclose: () => void } = $props();

  let phase = $state<"starting" | "recording" | "denied" | "unsupported">("starting");
  let seconds = $state(0);
  /** Recent input levels, 0-1, drawn as bars. */
  let levels = $state<number[]>(Array(32).fill(0));

  let recorder: MediaRecorder | null = null;
  let stream: MediaStream | null = null;
  let audioCtx: AudioContext | null = null;
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
    const analyser = audioCtx.createAnalyser();
    analyser.fftSize = 512;
    audioCtx.createMediaStreamSource(stream).connect(analyser);
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
  }

  function finish() {
    const blob = new Blob(chunks, { type: recorder?.mimeType || "audio/webm" });
    cleanup();
    if (!saving || blob.size === 0) return onclose();
    const ext = blob.type.includes("mp4") ? "m4a" : blob.type.includes("ogg") ? "ogg" : "webm";
    const stamp = new Date().toISOString().slice(0, 16).replace("T", " ").replace(":", ".");
    onsave(new File([blob], `Voice memo ${stamp}.${ext}`, { type: blob.type.split(";")[0] }));
  }

  function stop(save: boolean) {
    saving = save;
    if (recorder && recorder.state !== "inactive") recorder.stop();
    else onclose();
  }

  function cleanup() {
    clearInterval(timer);
    cancelAnimationFrame(frame);
    stream?.getTracks().forEach((t) => t.stop());
    void audioCtx?.close();
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

<Dialog title="Voice Memo" onclose={() => stop(false)}>
  <div class="recorder">
    {#if phase === "denied"}
      <p class="dim">Gnotes needs microphone access. Allow it in your browser's site settings and try again.</p>
    {:else if phase === "unsupported"}
      <p class="dim">This browser can't record audio here. Recording needs HTTPS (or localhost).</p>
    {:else}
      <div class="clock" class:live={phase === "recording"}>
        <span class="dot"></span>{clock}
      </div>
      <div class="levels" aria-hidden="true">
        {#each levels as l, i (i)}
          <span style:transform="scaleY({0.08 + l * 0.92})"></span>
        {/each}
      </div>
      <button class="stop" disabled={phase !== "recording"} aria-label="Stop and add to note" onclick={() => stop(true)}>
        <span class="square"></span>
      </button>
      <p class="dim hint">Tap to stop and add it to the note</p>
    {/if}
  </div>
  {#snippet actions()}
    <button onclick={() => stop(false)}><Icon name="close" /> Discard</button>
  {/snippet}
</Dialog>

<style>
  .recorder {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    padding: 4px 0 8px;
    text-align: center;
  }

  .clock {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 2.2rem;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .dot {
    width: 12px;
    height: 12px;
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
    gap: 3px;
    height: 56px;
    width: 100%;
    max-width: 300px;
  }

  .levels span {
    flex: 1;
    height: 100%;
    border-radius: var(--radius-sm);
    background: var(--accent-bg);
    transform-origin: center;
    transition: transform 80ms linear;
  }

  .stop {
    width: 76px;
    height: 76px;
    padding: 0;
    border-radius: 50%;
    background: var(--destructive-bg);
    box-shadow: 0 0 0 5px color-mix(in srgb, var(--destructive-bg) 22%, transparent);
  }

  .stop:active:not(:disabled) {
    background: var(--destructive-bg);
    transform: scale(0.93);
  }

  .square {
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
    background: #fff;
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
  }

  p {
    margin: 0;
    line-height: 1.45;
  }
</style>
