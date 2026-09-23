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
