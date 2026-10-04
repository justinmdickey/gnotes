<script lang="ts">
  import { ApiError, type ServiceInput, type ServiceSettings } from "./lib/api";
  import Icon from "./lib/Icon.svelte";
  import { ask, toast } from "./lib/ui.svelte";

  /**
   * Settings for an OpenAI-compatible service the server calls out to (speech-to-text, photo reading):
   * a row with a switch, like an AdwExpanderRow, that opens URL, model, optional key, an optional live
   * URL, and Test and Save. Folded away while the service is off.
   */
  let {
    name,
    title,
    subtitle,
    explain,
    urlPlaceholder,
    modelPlaceholder,
    live = false,
    settings = $bindable(),
    test,
    save,
    saved,
    off,
    reindex,
  }: {
    /** For the form's class, which tests and styles find it by. */
    name: string;
    title: string;
    /** One line under the title saying what it does. */
    subtitle: string;
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
    /** Offers Re-index while the service is on: makes its whole index again. */
    reindex?: () => Promise<string>;
  } = $props();

  let url = $state(settings.url);
  let model = $state(settings.model);
  let liveUrl = $state(settings.realtime_url ?? "");
  /** What's typed in the key field; empty means "keep the saved key". */
  let key = $state("");
  let clearKey = $state(false);
  let open = $state(settings.enabled);
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

  /** Puts the form back to what's saved. */
  function reset() {
    url = settings.url;
    model = settings.model;
    liveUrl = settings.realtime_url ?? "";
    key = "";
    clearKey = false;
    result = null;
  }

  /** On opens the form, and turns a service that was switched off back on with its saved settings.
   * Off stops the service but keeps them. */
  async function toggle() {
    if (!open) {
      open = true;
      if (settings.url && !settings.enabled) await apply({ url: settings.url, model: settings.model, realtime_url: settings.realtime_url, enabled: true });
      return;
    }
    if (settings.enabled) {
      const body: ServiceInput = { url: settings.url, model: settings.model, enabled: false };
      if (live) body.realtime_url = settings.realtime_url;
      if (!(await apply(body))) return;
    }
    reset();
    open = false;
  }

  async function runReindex() {
    const ok = await ask({
      title: "Re-index All Notes?",
      body: "Every note is sent to the service again. Ask may miss notes until it finishes.",
      confirm: "Re-index",
    });
    if (!ok || !reindex) return;
    try {
      toast(await reindex());
    } catch (err) {
      result = failure(err);
    }
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (await apply(input())) open = settings.enabled;
  }

  /** Saves a body, and shows what the server kept. An empty URL clears the service. */
  async function apply(body: ServiceInput) {
    try {
      settings = await save(body);
      reset();
      toast(settings.enabled ? saved : off);
      return true;
    } catch (err) {
      result = failure(err);
      return false;
    }
  }
</script>

<form class="boxed service {name}" class:open onsubmit={submit}>
  <div class="head">
    <div class="titles">
      <span class="title">{title}</span>
      <span class="dim subtitle">{subtitle}</span>
    </div>
    <button type="button" class="switch" role="switch" aria-checked={open} aria-label={title} onclick={toggle}></button>
  </div>
  {#if open}
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
  {#if reindex && settings.enabled}
    <div class="reindex">
      <span class="dim">Changed the model behind the same name? Build the index again.</span>
      <button type="button" onclick={runReindex}>Re-index</button>
    </div>
  {/if}
  {/if}
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

  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 34px;
    padding: 0 4px;
  }

  .titles {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .subtitle {
    font-size: var(--text-sm);
  }

  .service.open .head {
    padding-bottom: 10px;
    margin-bottom: 8px;
    border-bottom: 1px solid var(--border);
  }

  /* An AdwSwitch: a pill track with a round knob, accent when on. */
  .switch {
    flex: none;
    position: relative;
    width: 48px;
    min-width: 48px;
    height: 26px;
    min-height: 26px;
    padding: 0;
    border-radius: 13px;
    background: var(--border);
    transition: background 150ms;
  }

  .switch::after {
    content: "";
    position: absolute;
    top: 3px;
    left: 3px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgb(0 0 0 / 25%);
    transition: transform 150ms;
  }

  .switch[aria-checked="true"] {
    background: var(--accent-bg);
  }

  .switch[aria-checked="true"]::after {
    transform: translateX(22px);
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

  .reindex {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 12px;
    padding: 12px 4px 0;
    border-top: 1px solid var(--border);
    font-size: var(--text-sm);
  }

  .reindex span {
    flex: 1;
    min-width: 0;
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
