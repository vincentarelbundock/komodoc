<script>
  import { Tooltip } from "@skeletonlabs/skeleton-svelte";
  import Icon from "./Icon.svelte";

  // One button, one shape. Every icon control in the application is this, so
  // they are the same square with the icon at the same size, rather than each
  // place that wants one deciding again.
  //
  // Three weights, which is the whole vocabulary: filled for the one action
  // that changes the document, tonal for a state that is on, outlined for
  // everything else -- and plain for a control that sits inside something
  // else's frame, like a row of a table, where a border would draw a box
  // around nothing.
  //
  // An icon without a word beside it has to say what it is. A title attribute
  // says it only to a mouse, after a wait the browser chooses, in a box the
  // page has no say over; this says it to the keyboard too, promptly, in the
  // application's own colours.
  let {
    icon,
    label,
    title = label,
    href = null,
    pressed = null,
    disabled = false,
    tone = null,
    size = null,
    colour = null,
    filled = false,
    onclick,
  } = $props();

  const TONES = {
    filled: "preset-filled-primary-500",
    tonal: "preset-tonal-primary",
    outlined: "preset-outlined-surface-300-700",
    plain: "",
  };
  const preset = $derived(TONES[tone] ?? (pressed === true ? TONES.tonal : TONES.outlined));
  const classes = $derived(`btn-icon ${size ?? ""} ${preset} ${colour ?? ""}`);
</script>

<Tooltip openDelay={400} closeDelay={100} positioning={{ placement: "bottom" }}>
  <!-- The trigger is the control itself rather than a wrapper around it: an
       element in between would break the row the controls sit in. -->
  <Tooltip.Trigger>
    {#snippet element(attributes)}
      {#if href}
        <a {...attributes} {href} class={classes} aria-label={label} role="button">
          <Icon name={icon} {filled} />
        </a>
      {:else}
        <button
          {...attributes}
          type="button"
          class={classes}
          aria-label={label}
          aria-pressed={pressed === null ? undefined : pressed}
          {disabled}
          {onclick}
        >
          <Icon name={icon} {filled} />
        </button>
      {/if}
    {/snippet}
  </Tooltip.Trigger>
  <Tooltip.Positioner class="z-50">
    <Tooltip.Content class="card preset-filled-surface-950-50 px-2 py-1 text-xs shadow-lg">
      {title}
    </Tooltip.Content>
  </Tooltip.Positioner>
</Tooltip>
