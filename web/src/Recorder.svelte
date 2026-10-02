<script lang="ts">
  import { onDestroy } from "svelte";
  import Icon from "./lib/Icon.svelte";
  import { LiveSession, type LiveHandlers } from "./lib/live";
  import Menu, { type MenuItem } from "./lib/Menu.svelte";
  import { bloom, media, toast } from "./lib/ui.svelte";

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
  let audioCtx: AudioContext | null = null;
  /**
   * Every input feeds this one node: the microphone, and meeting audio when it's added. The saved
   * recording, the level bars and the live transcript all hear the mix, so inputs can change mid-way.
   */
  let mix: GainNode | null = null;
  let mic: { stream: MediaStream; node: MediaStreamAudioSourceNode } | null = null;
  let meeting = $state<{ stream: MediaStream; node: MediaStreamAudioSourceNode } | null>(null);
  let session: LiveSession | null = null;
  let chunks: Blob[] = [];
  let timer = 0;
  let frame = 0;
  let saving = false;

  /** Audio inputs the browser lists; their names show once the microphone is allowed. */
  let inputs = $state<MediaDeviceInfo[]>([]);
  let inputId = $state("");
  const MIC_KEY = "gnotes.mic";
  // Sharing a tab's or the screen's audio: Chrome and Edge on desktop. Firefox shares video only.
  const canShareAudio =
    !media.phone && !!navigator.mediaDevices?.getDisplayMedia && "suppressLocalAudioPlayback" in (navigator.mediaDevices.getSupportedConstraints?.() ?? {});

  /** Opus in WebM where supported (Chrome, Firefox), AAC in MP4 on Safari. */
  function pickType() {
    for (const t of ["audio/webm;codecs=opus", "audio/mp4", "audio/webm", "audio/ogg;codecs=opus"]) {
      if (MediaRecorder.isTypeSupported(t)) return t;
    }
    return "";
  }

  function savedInput() {
    try {
      return localStorage.getItem(MIC_KEY) ?? "";
    } catch {
      return "";
    }
  }

  /** Opens a microphone and puts it into the mix in place of the last. */
  async function useMic(deviceId: string, exact = false) {
    // A remembered input is only preferred, so a missing one falls back to the default; a pick is required.
    const want = deviceId ? { deviceId: exact ? { exact: deviceId } : { ideal: deviceId } } : {};
    const stream = await navigator.mediaDevices.getUserMedia({ audio: { ...want, echoCancellation: true, noiseSuppression: true } });
    if (!audioCtx || !mix) return void stream.getTracks().forEach((t) => t.stop());
    const node = audioCtx.createMediaStreamSource(stream);
    node.connect(mix);
    if (mic) {
      mic.node.disconnect();
      mic.stream.getTracks().forEach((t) => t.stop());
    }
    mic = { stream, node };
    inputId = exact ? deviceId : (stream.getAudioTracks()[0]?.getSettings().deviceId ?? deviceId);
    inputs = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === "audioinput" && d.deviceId);
  }

  async function chooseInput(deviceId: string) {
    if (deviceId === inputId) return;
    try {
      await useMic(deviceId, true);
      localStorage.setItem(MIC_KEY, deviceId);
    } catch {
      toast("Couldn't switch to that input");
    }
  }

  /** Asks to share a tab or the screen and mixes its sound in, for the other side of a call. */
  async function addMeeting() {
    let stream: MediaStream;
    try {
      // Browsers only share audio along with a picture; the picture is dropped right away.
      stream = await navigator.mediaDevices.getDisplayMedia({
        video: true,
        audio: { suppressLocalAudioPlayback: false },
        systemAudio: "include",
        selfBrowserSurface: "exclude",
      } as DisplayMediaStreamOptions);
    } catch {
      return;
    }
    stream.getVideoTracks().forEach((t) => t.stop());
    const track = stream.getAudioTracks()[0];
    if (!track || !audioCtx || !mix) {
      stream.getTracks().forEach((t) => t.stop());
      return toast("That share has no sound. Pick a tab and turn on “Share tab audio”.");
    }
    const node = audioCtx.createMediaStreamSource(stream);
    node.connect(mix);
    meeting = { stream, node };
    // The browser's own "Stop sharing" ends the track.
    track.addEventListener("ended", stopMeeting);
    toast("Meeting audio added. Use headphones so it isn't recorded twice.");
  }

  function stopMeeting() {
    meeting?.node.disconnect();
    meeting?.stream.getTracks().forEach((t) => t.stop());
    meeting = null;
  }

  const inputItems: MenuItem[] = $derived([
    ...inputs.map((d, i) => ({ label: d.label || `Input ${i + 1}`, checked: d.deviceId === inputId, onselect: () => chooseInput(d.deviceId) })),
    ...(canShareAudio
      ? [meeting
          ? { label: "Stop Meeting Audio", icon: "people" as const, onselect: stopMeeting }
          : { label: "Add Meeting Audio…", icon: "people" as const, onselect: () => void addMeeting() }]
      : []),
  ]);

  async function start() {
    if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") {
      phase = "unsupported";
      return;
    }
    // Made first, while the tap that started recording still counts, so it's allowed to run.
    audioCtx = new AudioContext();
    mix = audioCtx.createGain();
    try {
      await useMic(savedInput());
    } catch {
      phase = "denied";
      return;
    }
    void audioCtx.resume();
    const out = audioCtx.createMediaStreamDestination();
    mix.connect(out);
    const type = pickType();
    recorder = new MediaRecorder(out.stream, type ? { mimeType: type } : undefined);
    recorder.ondataavailable = (e) => e.data.size && chunks.push(e.data);
    recorder.onstop = finish;
    recorder.start(1000);
    phase = "recording";
    const began = performance.now();
    timer = window.setInterval(() => (seconds = Math.floor((performance.now() - began) / 1000)), 250);

    const analyser = audioCtx.createAnalyser();
    analyser.fftSize = 512;
    mix.connect(analyser);
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
        session = await LiveSession.start(audioCtx, mix, {
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
    stopMeeting();
    mic?.stream.getTracks().forEach((t) => t.stop());
    void audioCtx?.close();
    session = null;
    mic = null;
    mix = null;
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
    {#if meeting}<span class="tag on" title="Sound from the shared tab or screen is mixed in">Meeting</span>{/if}
    {#if liveState}
      <span class="tag" class:on={liveState === "on"} title={liveState === "off" ? "Live transcript stopped; it will be transcribed when saved" : "Words appear in the note as you speak"}>
        {liveState === "off" ? "Not live" : "Live"}
      </span>
    {/if}
    {#if inputItems.length > 1 || canShareAudio}
      <Menu label="Audio input" class="flat icon circular input" items={inputItems}>
        {#snippet trigger()}<Icon name="mic" />{/snippet}
      </Menu>
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
