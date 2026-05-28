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
    place-items: center;
    border: 1px solid #e4e4e7;
    border-radius: 8px;
    background: #ffffff;
    color: #71717a;
  }

  .company-tab-placeholder p {
    margin: 0;
    font-size: 13px;
    line-height: 18px;
  }
</style>
