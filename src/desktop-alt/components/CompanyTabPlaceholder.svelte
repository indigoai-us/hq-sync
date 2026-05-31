<script lang="ts">
  interface Props {
    label: string;
  }

  let { label }: Props = $props();
</script>

<section class="company-tab-placeholder" aria-label={label}>
  <p>{label}</p>
</section>

<style>
  .company-tab-placeholder {
    display: grid;
    min-height: 260px;
    min-width: 0;
    place-items: center;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    color: var(--muted);
  }

  .company-tab-placeholder p {
    margin: 0;
    max-width: 100%;
    overflow: hidden;
    padding: 0 16px;
    font-size: 13px;
    line-height: 18px;
    text-align: center;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (max-width: 520px) {
    .company-tab-placeholder p {
      white-space: normal;
    }
  }
</style>
