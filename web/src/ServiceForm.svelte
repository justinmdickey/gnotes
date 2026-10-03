<script lang="ts">
  import { ApiError, type ServiceInput, type ServiceSettings } from "./lib/api";
  import Icon from "./lib/Icon.svelte";
  import { toast } from "./lib/ui.svelte";

  /**
   * Settings for an OpenAI-compatible service the server calls out to (speech-to-text, photo reading):
   * URL, model, optional key, an optional live URL, and Test and Save.
   */
  let {
    name,
    explain,
    urlPlaceholder,
    modelPlaceholder,
    live = false,
    settings = $bindable(),
    test,
    save,
    saved,
    off,
  }: {
    /** For the form's class, which tests and styles find it by. */
    name: string;
    explain: string;
    urlPlaceholder: string;
    modelPlaceholder: string;
    /** Shows the live (websocket) URL field. */
    live?: boolean;
    settings: ServiceSettings;
    test: (body: ServiceInput) => Promise<{ ok: boolean; message: string }>;
    save: (body: ServiceInput) => Promise<ServiceSettings>;
    /** Toasts after saving, and after turning it off. */
    saved: string;
    off: string;
  } = $props();

  let url = $state(settings.url);
  let model = $state(settings.model);
  let liveUrl = $state(settings.realtime_url ?? "");
  /** What's typed in the key field; empty means "keep the saved key". */
  let key = $state("");
  let clearKey = $state(false);
  let testing = $state(false);
  let result = $state<{ ok: boolean; message: string } | null>(null);
  const dirty = $derived(
    url.trim() !== settings.url ||
      model.trim() !== settings.model ||
      (live && liveUrl.trim() !== (settings.realtime_url ?? "")) ||
      key !== "" ||
      clearKey,
  );

  function input(): ServiceInput {
    const body: ServiceInput = { url, model };
    if (live) body.realtime_url = liveUrl;
    if (key) body.key = key;
    else if (clearKey) body.key = null;
    return body;
  }

  const failure = (err: unknown) => ({ ok: false, message: err instanceof ApiError ? err.message : "Couldn't reach the server" });

  async function runTest() {
    testing = true;
    result = null;
    try {
      result = await test(input());
    } catch (err) {
      result = failure(err);
    }
    testing = false;
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    const turningOff = !url.trim();
    try {
      settings = await save(input());
      url = settings.url;
      model = settings.model;
      liveUrl = settings.realtime_url ?? "";
      key = "";
      clearKey = false;
      result = null;
      toast(turningOff ? off : saved);
    } catch (err) {
      result = failure(err);
    }
  }
</script>

<form class="boxed service {name}" onsubmit={submit}>
  <p class="dim explain">{explain}</p>
  <label class="field">
    <span>Service URL</span>
    <input type="url" placeholder={urlPlaceholder} autocapitalize="none" spellcheck="false" bind:value={url} />
  </label>
  <label class="field">
    <span>Model</span>
    <input placeholder={modelPlaceholder} autocapitalize="none" spellcheck="false" bind:value={model} />
  </label>
  {#if live}
    <label class="field">
      <span>Live URL</span>
      <input type="url" placeholder="Optional, e.g. ws://whisper:8000/v1/realtime" autocapitalize="none" spellcheck="false" bind:value={liveUrl} />
    </label>
  {/if}
  <label class="field">
    <span>API key</span>
    <input
      type="password"
      autocomplete="off"
      placeholder={settings.has_key && !clearKey ? "Saved – type to replace" : "Optional"}
      bind:value={key}
      oninput={() => (clearKey = false)}
    />
    {#if settings.has_key && !clearKey && !key}
      <button type="button" class="flat destructive clear" onclick={() => (clearKey = true)}>Remove</button>
    {/if}
  </label>
  <div class="foot">
    <span class="status" class:ok={result?.ok} class:bad={result && !result.ok}>
      {#if testing}
        <span class="spinner"></span> Testing…
      {:else if result}
        <Icon name={result.ok ? "check" : "close"} /> {result.message}
      {:else if settings.enabled}
        <span class="dot on"></span> On{settings.from_env ? " (from the server's environment)" : ""}
      {:else}
        <span class="dot"></span> Off
      {/if}
    </span>
    <button type="button" disabled={!url.trim() || testing} onclick={runTest}>Test</button>
    <button type="submit" class="suggested" disabled={!dirty}>Save</button>
  </div>
</form>

<style>
  .boxed {
    margin: 0;
    border-radius: var(--radius-lg);
    background: var(--card-bg);
    box-shadow: var(--shadow-sm), 0 0 0 1px var(--border);
    overflow: hidden;
  }

  .service {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px;
  }

  .explain {
    margin: 0 4px 8px;
    font-size: var(--text-sm);
    line-height: 1.45;
  }

  .field {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 46px;
    padding: 0 4px;
  }

  .field + .field {
    border-top: 1px solid var(--border);
  }

  .field span {
    flex: none;
    width: 92px;
  }

  .field input {
    flex: 1;
    min-width: 0;
  }

  .clear {
    min-height: 32px;
    padding: 0 10px;
  }

  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
    padding: 0 4px;
  }

  .status {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--text-sm);
    color: var(--dim-fg);
  }

  .status.ok {
    color: var(--success);
  }

  .status.bad {
    color: var(--destructive);
  }

  .status .spinner {
    width: 14px;
    height: 14px;
  }

  .status .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--dim-fg);
  }

  .status .dot.on {
    background: var(--success-bg);
  }

  @media (max-width: 700px) {
    .field {
      flex-wrap: wrap;
      gap: 4px 12px;
      padding: 8px 4px;
    }

    .field span {
      width: 100%;
      font-size: var(--text-sm);
      color: var(--dim-fg);
    }
  }
</style>
