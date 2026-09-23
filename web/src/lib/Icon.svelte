<script lang="ts" module>
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
    newfolder: "M2.5 4.5v8h11v-6.5h-6l-1.5-1.5zM8 7.5v4M6 9.5h4",
    person_add: "M6.5 7a2.2 2.2 0 1 0 0-4.4 2.2 2.2 0 0 0 0 4.4zM2.5 13c0-2.2 1.8-3.8 4-3.8 1 0 1.9.3 2.6.9M12 9v4.5M9.8 11.2h4.4",
    link: "M7 9a2.5 2.5 0 0 0 3.5 0l2-2a2.5 2.5 0 0 0-3.5-3.5l-.7.7M9 7a2.5 2.5 0 0 0-3.5 0l-2 2A2.5 2.5 0 0 0 7 12.5l.7-.7",
    compose: "M13 8.5v4.5H3V3h4.5M7 9l.4-2L12 2.4 13.6 4 9 8.6z",
    search: "M7 11.5a4.5 4.5 0 1 0 0-9 4.5 4.5 0 0 0 0 9zM10.3 10.3 13.5 13.5",
    checklist: "M2.5 4.5l1 1 2-2M2.5 10.5l1 1 2-2M8 4.5h5.5M8 10.5h5.5",
    bullets: "M3.5 4.5h.01M3.5 8h.01M3.5 11.5h.01M6.5 4.5h7M6.5 8h7M6.5 11.5h7",
    numbers: "M2.5 3.5h1v3M2.5 9.5h2l-2 2.5h2M6.5 4.5h7M6.5 8h7M6.5 11.5h7",
    quote: "M3 3.5v9M6 5h7M6 8h7M6 11h5",
    code: "M5.5 4.5 2 8l3.5 3.5M10.5 4.5 14 8l-3.5 3.5",
    textformat: "M2 12.5 5 4l3 8.5M3 10h4M10.5 8.5c.5-.7 1.2-1 2-1 1 0 1.5.6 1.5 1.5v3.5M14 10.5c-2.5-.5-4 .2-4 1.2 0 .6.5 1 1.2 1 1.3 0 2.3-.8 2.8-1.7",
    sidebar: "M2.5 3.5h11v9h-11zM6 3.5v9",
    close: "M4 4l8 8M12 4l-8 8",
    check: "M3.5 8.5l3 3 6-7",
    next: "M6 3l5 5-5 5",
    key: "M5.5 10.5a3 3 0 1 1 2.1-.9L13.5 3.7M11.5 5.7l1.5 1.5M9.8 7.4l1.2 1.2",
    devices: "M2.5 3.5h8v6h-8zM4.5 12.5h4M6.5 9.5v3M11.5 6.5h2v6h-2z",
    rename: "M3 13l.5-2.5 7-7 2 2-7 7zM9.5 4.5l2 2",
    shield: "M8 2.5l5 2v3.5c0 3-2.2 5-5 5.5-2.8-.5-5-2.5-5-5.5V4.5z",
    user: "M8 7.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5zM3 13.5c0-2.5 2.2-4.2 5-4.2s5 1.7 5 4.2",
    camera: "M2.5 5.5h2.5l1-1.5h4l1 1.5h2.5v7.5h-11zM8 11a2 2 0 1 0 0-4 2 2 0 0 0 0 4z",
    mic: "M8 2.5a2 2 0 0 0-2 2V8a2 2 0 1 0 4 0V4.5a2 2 0 0 0-2-2zM4 7.5a4 4 0 0 0 8 0M8 11.5v2",
    image: "M2.5 3.5h11v9h-11zM2.5 10.5l3-3 3 3 2-2 3 3M10.5 6.5h.01",
  };
  export type IconName = keyof typeof paths;
</script>

<script lang="ts">
  let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<svg class="icon" viewBox="0 0 16 16" width={size} height={size} aria-hidden="true">
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
