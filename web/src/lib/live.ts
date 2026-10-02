// Live transcripts while recording. Microphone audio goes to the server as 16 kHz mono PCM over
// /api/transcribe/live, which relays it to the realtime speech-to-text service and sends text back.

export interface LiveHandlers {
  /** More words for the stretch of speech being heard. */
  ondelta: (text: string) => void;
  /** The finished text for the stretch so far; it replaces that stretch's deltas. */
  onfinal: (text: string) => void;
  /** The service failed or went away; the recording itself carries on. */
  onerror: (message: string) => void;
}

// Runs on the audio thread: averages the input down to 16 kHz and posts 100 ms chunks of 16-bit PCM.
const WORKLET = `
class Pcm16k extends AudioWorkletProcessor {
  constructor() {
    super();
    this.step = sampleRate / 16000;
    this.pos = 0; this.sum = 0; this.count = 0;
    this.out = new Int16Array(1600); this.n = 0;
  }
  process(inputs) {
    const input = inputs[0][0];
    if (!input) return true;
    for (let i = 0; i < input.length; i++) {
      this.sum += input[i]; this.count++;
      if (++this.pos >= this.step) {
        this.pos -= this.step;
        const v = Math.max(-1, Math.min(1, this.sum / this.count));
        this.sum = 0; this.count = 0;
        this.out[this.n++] = v * 0x7fff;
        if (this.n === this.out.length) {
          this.port.postMessage(this.out.buffer, [this.out.buffer]);
          this.out = new Int16Array(1600); this.n = 0;
        }
      }
    }
    return true;
  }
}
registerProcessor("pcm16k", Pcm16k);
`;

export class LiveSession {
  private ws: WebSocket;
  private node: AudioWorkletNode | null = null;
  /** Audio captured before the server said it's ready. */
  private queued: ArrayBuffer[] = [];
  private ready = false;
  private failed = false;
  private finished: (() => void) | null = null;

  private constructor(
    private source: AudioNode,
    private on: LiveHandlers,
  ) {
    const scheme = location.protocol === "https:" ? "wss" : "ws";
    this.ws = new WebSocket(`${scheme}://${location.host}/api/transcribe/live`);
    this.ws.onmessage = (e) => this.message(JSON.parse(e.data));
    this.ws.onclose = () => {
      if (!this.ready && !this.failed) this.fail("Couldn't start live transcription");
      this.finished?.();
    };
  }

  /** Starts streaming `source` (an input of `ctx`). Resolves once audio is flowing to the server. */
  static async start(ctx: AudioContext, source: AudioNode, on: LiveHandlers): Promise<LiveSession> {
    const session = new LiveSession(source, on);
    const url = URL.createObjectURL(new Blob([WORKLET], { type: "text/javascript" }));
    try {
      await ctx.audioWorklet.addModule(url);
    } finally {
      URL.revokeObjectURL(url);
    }
    session.node = new AudioWorkletNode(ctx, "pcm16k");
    session.node.port.onmessage = (e: MessageEvent<ArrayBuffer>) => session.send(e.data);
    source.connect(session.node);
    return session;
  }

  get ok() {
    return !this.failed;
  }

  private send(chunk: ArrayBuffer) {
    if (this.failed) return;
    if (!this.ready) this.queued.push(chunk);
    else if (this.ws.readyState === WebSocket.OPEN) this.ws.send(chunk);
  }

  private message(m: { t: string; text?: string; message?: string }) {
    if (m.t === "ready") {
      this.ready = true;
      for (const chunk of this.queued) this.ws.send(chunk);
      this.queued = [];
    } else if (m.t === "delta" && m.text) this.on.ondelta(m.text);
    else if (m.t === "final") {
      this.on.onfinal(m.text ?? "");
      this.finished?.();
    } else if (m.t === "error") this.fail(m.message ?? "Live transcription failed");
  }

  private fail(message: string) {
    if (this.failed) return;
    this.failed = true;
    this.on.onerror(message);
  }

  /** Stops listening, waits (briefly) for the last finished text, and closes. */
  async stop() {
    if (this.node) this.source.disconnect(this.node);
    this.node?.disconnect();
    if (this.ready && !this.failed && this.ws.readyState === WebSocket.OPEN) {
      const done = new Promise<void>((r) => (this.finished = r));
      this.ws.send(JSON.stringify({ t: "commit" }));
      await Promise.race([done, new Promise((r) => setTimeout(r, 5000))]);
    }
    this.close();
  }

  close() {
    this.finished = null;
    this.node?.disconnect();
    if (this.ws.readyState <= WebSocket.OPEN) this.ws.close();
  }
}
