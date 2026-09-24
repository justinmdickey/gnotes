<script lang="ts" module>
  export type { IconName } from "./icons";
</script>

<script lang="ts">
  import { iconPath, type IconName } from "./icons";

  let { name, size = 16 }: { name: IconName; size?: number } = $props();
  const icon = $derived(iconPath(name));
</script>

<svg class="icon" class:filled={icon.filled} viewBox="0 0 16 16" width={size} height={size} aria-hidden="true">
  <path d={icon.d} />
</svg>
