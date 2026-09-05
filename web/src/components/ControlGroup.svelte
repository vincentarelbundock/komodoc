<script>
  // A row of controls that belong together: the panes, and nothing else so
  // far. It is a group for what it says about them -- these are one choice,
  // read left to right -- and not for how they look, so it takes no part in
  // sizing them.
  let { label, children } = $props();
</script>

<div class="group" role="group" aria-label={label}>
  {@render children()}
</div>

<style>
  .group {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    /* Pico styles [role=group] as a segmented control: joined corners, its own
       margins, and a height of its own. None of that is wanted here, and its
       margin is what pushed this row off the line the rest of the bar sits on. */
    margin: 0;
    box-shadow: none;
  }
  /* Pico also makes the children of a group share the width between them,
     which stretched each square control into a rectangle. They are the size
     they are; the group only decides that they sit together. */
  .group > :global(*) {
    flex: 0 0 auto;
  }
</style>
