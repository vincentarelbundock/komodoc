<script>
  // A separator that can be dragged, or focused and moved with the arrow keys.
  //
  // While dragging, only a guide line moves; the real width -- and the iframe
  // reflow that comes with it -- is applied once, on release.
  let { pane, label, onwidth, onguide, ongrab } = $props();

  let element = $state(null);

  function down(event) {
    event.preventDefault();
    element.setPointerCapture(event.pointerId);
    ongrab?.(true);
    onguide?.(pane.widthAt(event.clientX));
  }

  function move(event) {
    if (!element.hasPointerCapture(event.pointerId)) return;
    onguide?.(pane.widthAt(event.clientX));
  }

  function finish(event) {
    if (!element.hasPointerCapture(event.pointerId)) return;
    element.releasePointerCapture(event.pointerId);
    ongrab?.(false);
    onwidth?.(pane.widthAt(event.clientX));
  }

  function key(event) {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    onwidth?.(null, event.key === pane.grows ? 24 : -24);
  }
</script>

<div
  bind:this={element}
  class="grip"
  role="separator"
  aria-orientation="vertical"
  aria-label={label}
  tabindex="0"
  onpointerdown={down}
  onpointermove={move}
  onpointerup={finish}
  onpointercancel={finish}
  onkeydown={key}
></div>
