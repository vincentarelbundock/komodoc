<script>
  import Icon from "./Icon.svelte";

  // The address bar already holds the link; the button spares the reader from
  // selecting it. The icon turns into a tick for a moment, since a copy is
  // otherwise invisible.
  let { href = null, label = "Copy link", classes = "outline iconbtn" } = $props();
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

<button type="button" class="{classes}{done ? ' done' : ''}" onclick={copy}
        aria-label={label} title={label}>
  <Icon name={done ? "check" : "link"} />
</button>
