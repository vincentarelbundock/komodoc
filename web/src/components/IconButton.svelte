<script>
  import Icon from "./Icon.svelte";

  // One button, one shape. Every icon control on the bar is this, so they are
  // the same square, on the same line, with the icon at the same size --
  // rather than each place that wants one repeating a string of classes and
  // the stylesheet reaching down through whatever ancestor it happens to
  // have. That is what had three of them sitting on three different lines.
  //
  // A control that navigates is a link and a control that acts is a button.
  // Both are drawn here, so the two cannot end up looking different.
  let {
    icon,
    label,
    title = label,
    href = null,
    pressed = null,
    disabled = false,
    tone = "outline",
    onclick,
  } = $props();
</script>

{#if href}
  <a {href} class="control {tone}" aria-label={label} {title} role="button">
    <Icon name={icon} />
  </a>
{:else}
  <button
    type="button"
    class="control {tone}"
    aria-label={label}
    {title}
    aria-pressed={pressed === null ? undefined : pressed}
    {disabled}
    {onclick}
  >
    <Icon name={icon} />
  </button>
{/if}

<style>
  /* The size of every control on the bar, and the only place it is decided.
     Pico gives a button inside a [role=group] different metrics from a loose
     one, which is what had the pane toggles sitting eight pixels above their
     neighbours. */
  .control {
    box-sizing: border-box;
    /* Never shrinks: a control is the size it is, and a row that is short of
       room scrolls rather than squeezing its buttons into slivers. */
    flex: none;
    width: var(--komodoc-control, 2rem);
    height: var(--komodoc-control, 2rem);
    padding: 0;
    margin: 0;
    display: inline-grid;
    place-items: center;
    line-height: 1;
    border-radius: var(--pico-border-radius);
    border: 1px solid transparent;
    cursor: pointer;
  }
  .control:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  /* Outlined by default: an icon on the bar is an option, not a call to
     action, so it carries the weight of one. */
  .outline {
    background: transparent;
    border-color: var(--pico-muted-border-color);
    color: var(--pico-color);
  }
  .outline:hover:not(:disabled) {
    border-color: var(--pico-primary);
    color: var(--pico-primary);
  }
  /* Pressed is a state of the page rather than of the pointer, so it stays
     lit once it is. */
  .outline[aria-pressed="true"] {
    border-color: var(--pico-primary);
    color: var(--pico-primary);
    background: color-mix(in srgb, var(--pico-primary) 10%, transparent);
  }
  /* The one control on the bar that changes the document rather than the view
     of it. */
  .primary {
    background: var(--pico-primary-background);
    border-color: var(--pico-primary-border);
    color: var(--pico-primary-inverse);
  }
  .primary:hover:not(:disabled) {
    background: var(--pico-primary-hover-background);
  }
  .primary:disabled {
    background: var(--pico-secondary-background);
    border-color: var(--pico-secondary-border);
  }
  /* A copy leaves nothing visible behind, so the icon itself is the receipt. */
  .done {
    border-color: var(--pico-ins-color);
    color: var(--pico-ins-color);
  }
  a.control {
    text-decoration: none;
  }
</style>
