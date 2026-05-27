<script lang="ts">
  interface Props {
    version: string;
  }

  let { version }: Props = $props();
</script>

<footer class="desktop-status-bar" aria-label="Status">
  <span>Ready</span>
  <span>v{version}</span>
</footer>
