<script lang="ts">
  /**
   * MessagesPage — the desktop-alt Messages surface (a top-level destination,
   * ⌘4). A thin host that mounts the SAME <MessagesShell/> used by the
   * standalone Messages window (one implementation, two hosts — US-019). The
   * shell owns the segmented rail (DMs / Requests / Channels), the shared
   * <Conversation> pane, and channels; this page only gives it a full-bleed box
   * to live in inside the desktop shell.
   */
  import MessagesShell from '../../components/messaging/MessagesShell.svelte';
</script>

<div class="messages-page" data-testid="desktop-alt-messages">
  <MessagesShell />
</div>

<style>
  /* Full-bleed host. The shell's own root is sized to the viewport (it was
     authored as a standalone window); inside the desktop content column we pin
     this host to the content area and clamp the shell's root to fill the host
     instead of the whole viewport — same component, hosted not forked. */
  .messages-page {
    position: absolute;
    inset: 0;
    display: flex;
    overflow: hidden;
  }

  .messages-page :global(.messages-window) {
    width: 100%;
    height: 100%;
  }
</style>
