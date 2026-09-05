<script>
  import Icon from "./Icon.svelte";
  import CommentCard from "./CommentCard.svelte";

  // The annotations, in document order: an annotation is about a place in the
  // text, so the column follows the page rather than the order things were
  // written. A note on a figure sorts by where that figure sits in the text.
  // Anything that could not be anchored has no place to sort by, so it goes to
  // the end, in the order it was made.
  let {
    comments = [],
    figureAt = [],
    identity = "",
    canModerate = false,
    tool = "commenting",
    hasFigures = false,
    ontool,
    onreveal,
    onresolve,
    ondelete,
    onreply,
  } = $props();

  // An empty set shows everything; an annotation matches if it carries every
  // tag chosen.
  let chosen = $state(new Set());

  const allTags = $derived.by(() => {
    const all = new Set();
    for (const comment of comments) for (const tag of comment.tags || []) all.add(tag);
    return [...all].sort();
  });

  function toggleTag(tag) {
    const next = new Set(chosen);
    next.has(tag) ? next.delete(tag) : next.add(tag);
    chosen = next;
  }

  function place(comment) {
    if (comment.region) {
      const at = figureAt[comment.region.image_index];
      if (Number.isFinite(at)) return at;
    }
    return Number.isFinite(comment.start) ? comment.start : Infinity;
  }

  const shown = $derived(
    comments
      .filter((comment) => [...chosen].every((tag) => (comment.tags || []).includes(tag)))
      .sort((a, b) => place(a) - place(b) || a.seq - b.seq),
  );

  const open = $derived(comments.filter((comment) => !comment.resolved).length);

  const TOOLS = [
    { id: "commenting", icon: "comment", label: "Comment", title: "Comment on the selected passage" },
    { id: "highlighting", icon: "highlight", label: "Highlight", title: "Highlight, with no comment" },
    { id: "region", icon: "box", label: "Box", title: "Drag a box on a figure" },
  ];
</script>

<aside class="sidebar">
  <!-- The tools belong with the comments they make, and stay in view while the
       pane scrolls. Box draws on a figure, so a document with no figures has
       nothing for it to do; saying so is better than a button that silently
       does nothing. -->
  <div class="tools" role="radiogroup" aria-label="Annotation tool">
    {#each TOOLS as item}
      <button
        type="button"
        class="tool"
        class:outline={tool !== item.id}
        aria-pressed={tool === item.id}
        aria-label={item.label}
        disabled={item.id === "region" && !hasFigures}
        title={item.id === "region" && !hasFigures
          ? "This document has no figures to draw on"
          : item.title}
        onclick={() => ontool?.(item.id)}
      >
        <Icon name={item.icon} />
        <span>{item.label}</span>
      </button>
    {/each}
  </div>

  <hgroup>
    <h3>Comments <small>{comments.length ? `${open} open · ${comments.length} total` : ""}</small></h3>
    <!-- Onboarding, not a caption: it goes once there is something in the
         column to read. -->
    {#if comments.length === 0}
      <p>Highlight text in the document, then choose “Comment”.</p>
    {/if}
  </hgroup>

  {#if allTags.length}
    <div class="tagfilter">
      {#each allTags as tag}
        <button type="button" class="tag" class:chosen={chosen.has(tag)} onclick={() => toggleTag(tag)}>
          {tag}
        </button>
      {/each}
    </div>
  {/if}

  <div id="comments">
    {#each shown as comment (comment)}
      <CommentCard
        {comment}
        {identity}
        {canModerate}
        ontoggleTag={toggleTag}
        {onreveal}
        {onresolve}
        {ondelete}
        {onreply}
      />
    {/each}
  </div>
</aside>
