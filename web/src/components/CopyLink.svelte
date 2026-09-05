<script>
  import IconButton from "./IconButton.svelte";

  // The address bar already holds the link; the button spares the reader from
  // selecting it. The icon turns into a tick for a moment, since a copy is
  // otherwise invisible.
  let { href = null, label = "Copy link" } = $props();
  let done = $state(false);
  let timer;

  async function copy() {
    try {
      await navigator.clipboard.writeText(href ?? location.href);
    } catch {
      return; // clipboard blocked; the address bar is still there
    }
    done = true;
    clearTimeout(timer);
    timer = setTimeout(() => (done = false), 1500);
  }
</script>

<IconButton
  icon={done ? "check" : "link"}
  {label}
  tone={done ? "outline done" : "outline"}
  onclick={copy}
/>
