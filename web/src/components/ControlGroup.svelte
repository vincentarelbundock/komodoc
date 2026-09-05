<script>
  // A row of controls that belong together: the panes, and nothing else so
  // far. It is a group for what it says about them -- these are one choice,
  // read left to right -- and takes no part in how they look.
  let { label, children } = $props();
</script>

<div class="flex items-center gap-1" role="group" aria-label={label}>
  {@render children()}
</div>
