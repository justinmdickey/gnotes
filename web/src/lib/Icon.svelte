<script lang="ts">
  // Minimal symbolic icons drawn in the Adwaita style (16px grid, 1.5px strokes).
  const paths = {
    plus: "M8 3v10M3 8h10",
    back: "M10 3 5 8l5 5",
    menu: "M3 4.5h10M3 8h10M3 11.5h10",
    share: "M5.5 7a2 2 0 1 0 0 2M10.5 4.5a2 2 0 1 0 0-.01M10.5 11.5a2 2 0 1 0 0-.01M7.3 7.1l1.6-1.2M7.3 8.9l1.6 1.2",
    trash: "M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 8.5h5.6l.7-8.5",
    folder: "M2.5 4.5v8h11v-6.5h-6l-1.5-1.5z",
    note: "M4 2.5h5.5l2.5 2.5v8.5H4zM9.5 2.5V5H12",
    people: "M6 7a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM2.5 13c0-2 1.5-3.5 3.5-3.5s3.5 1.5 3.5 3.5M10.5 7a1.7 1.7 0 1 0 0-3.4M11.5 9.5c1.3.3 2 1.6 2 3.5",
    pencil: "M3 13l.5-2.5 7-7 2 2-7 7zM9.5 4.5l2 2",
    logout: "M9.5 3h-6v10h6M7 8h6.5M11 5.5 13.5 8 11 10.5",
    more: "M7.6 3.5h.8M7.6 8h.8M7.6 12.5h.8",
    compose: "M13 8.5v4.5H3V3h4.5M7 9l.4-2L12 2.4 13.6 4 9 8.6z",
    search: "M7 11.5a4.5 4.5 0 1 0 0-9 4.5 4.5 0 0 0 0 9zM10.3 10.3 13.5 13.5",
    checklist: "M2.5 4.5l1 1 2-2M2.5 10.5l1 1 2-2M8 4.5h5.5M8 10.5h5.5",
    bullets: "M3.5 4.5h.01M3.5 8h.01M3.5 11.5h.01M6.5 4.5h7M6.5 8h7M6.5 11.5h7",
    numbers: "M2.5 3.5h1v3M2.5 9.5h2l-2 2.5h2M6.5 4.5h7M6.5 8h7M6.5 11.5h7",
    quote: "M3 3.5v9M6 5h7M6 8h7M6 11h5",
    code: "M5.5 4.5 2 8l3.5 3.5M10.5 4.5 14 8l-3.5 3.5",
    textformat: "M2 12.5 5 4l3 8.5M3 10h4M10.5 8.5c.5-.7 1.2-1 2-1 1 0 1.5.6 1.5 1.5v3.5M14 10.5c-2.5-.5-4 .2-4 1.2 0 .6.5 1 1.2 1 1.3 0 2.3-.8 2.8-1.7",
  };
  let { name }: { name: keyof typeof paths } = $props();
</script>

<svg class="icon" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
  <path d={paths[name]} />
</svg>

<style>
  .icon {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
