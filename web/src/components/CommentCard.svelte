<script>
  import { AUTHOR, read, write } from "../lib/storage.js";

  // One annotation, and everything said about it.
  let {
    comment,
    identity = "",
    canModerate = false,
    ontoggleTag,
    onreveal,
    onresolve,
    ondelete,
    onreply,
  } = $props();

  // A passage can be a paragraph long, which would bury the comment made about
  // it. The card shows the opening words and expands on request.
  const QUOTE_WORDS = 8;

  // A resolved note is settled business: it collapses to one line and stays
  // out of the way until someone clicks it open again. Resolving it again
  // closes it, however it was left.
  let expanded = $state(false);
  let quoteOpen = $state(false);
  let replying = $state(false);
  let replyBody = $state("");
  let replyName = $state(identity || read(AUTHOR, "Anonymous"));

  const collapsed = $derived(Boolean(comment.resolved) && !expanded);
  $effect(() => {
    if (!comment.resolved) expanded = false;
  });

  const words = $derived((comment.exact || "").split(/\s+/));
  const long = $derived(words.length > QUOTE_WORDS + 2);
  const short = $derived(words.slice(0, QUOTE_WORDS).join(" "));

  // The Delete button is only ever real for a comment the server says this
  // caller may delete, one they just posted and is still waiting to be
  // confirmed, or a caller who owns the document outright.
  const deletable = $derived(
    Boolean(comment.deletable) || Boolean(comment.temp_id) || Boolean(comment.pending) || canModerate,
  );

  const stamp = (value) => (value || "").replace("T", " ").slice(0, 16) + " UTC";

  // The one line a resolved card shows: what it was about, then what was said
  // about it. Trimmed to a line by CSS rather than by cutting the text, so the
  // width of the column decides how much of it fits.
  const summary = $derived(
    [
      comment.region ? `Figure ${comment.region.image_index + 1}` : (comment.exact || "").trim(),
      (comment.body || "").trim(),
    ]
      .filter(Boolean)
      .join(" — ") || "Resolved",
  );

  function click(event) {
    if (event.target.closest("button,input,textarea,a")) return;
    // Collapsed, the click is "show me this again"; open, it is "take me to
    // the place in the document this is about".
    if (collapsed) {
      expanded = true;
      return;
    }
    if (!comment.orphaned) onreveal?.(comment);
  }

  function submitReply(event) {
    event.preventDefault();
    if (!replyBody.trim()) return;
    if (!identity) write(AUTHOR, replyName);
    onreply?.(comment, replyBody, replyName);
    replyBody = "";
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<article id="comment-{comment.id}"
         class:resolved={comment.resolved || comment.pending}
         class:collapsed
         onclick={click}>
  {#if collapsed}
    <div class="summary">{summary}</div>
  {:else}
    {#if comment.orphaned}<mark>Needs re-anchoring</mark>{/if}
    <!-- The motivation is the W3C annotation type. Commenting is the default,
         so only the others are worth showing. -->
    {#if comment.motivation && comment.motivation !== "commenting"}
      <mark data-motivation={comment.motivation}>{comment.motivation}</mark>
    {/if}

    {#if comment.region}
      <blockquote class="figureref">Figure {comment.region.image_index + 1}</blockquote>
    {:else if long}
      <blockquote>
        <span>{quoteOpen ? `“${comment.exact}”` : `“${short}`}</span>
        <!-- svelte-ignore a11y_invalid_attribute -->
        <a href="#" onclick={(e) => { e.preventDefault(); e.stopPropagation(); quoteOpen = !quoteOpen; }}>
          {quoteOpen ? " less" : "… ”"}
        </a>
      </blockquote>
    {:else}
      <blockquote>“{comment.exact}”</blockquote>
    {/if}

    {#if comment.body}<p>{comment.body}</p>{/if}
    {#each comment.tags || [] as tag}
      <button type="button" class="tag" onclick={(e) => { e.stopPropagation(); ontoggleTag?.(tag); }}>{tag}</button>
    {/each}
    <small>{comment.creator} · {stamp(comment.created)}</small>

    {#if comment.replies?.length}
      <ul>
        {#each comment.replies as reply (reply.id)}
          <li>
            <span>{reply.body}</span><br />
            <small>{reply.creator} · {stamp(reply.created)}</small>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}

  <!-- Not role="group": that is Pico's segmented control, which joins its
       buttons into one shape. These are separate actions. -->
  <div class="actions">
    <button type="button" onclick={(e) => { e.stopPropagation(); expanded = false; onresolve?.(comment); }}>
      {comment.resolved ? "Reopen" : "Resolve"}
    </button>
    <button type="button" onclick={(e) => { e.stopPropagation(); replying = !replying; }}>Reply</button>
    {#if deletable}
      <button type="button" onclick={(e) => { e.stopPropagation(); ondelete?.(comment); }}>Delete</button>
    {/if}
  </div>

  {#if replying}
    <form onsubmit={submitReply}>
      {#if !identity}
        <input placeholder="Name" maxlength="80" bind:value={replyName} />
      {/if}
      <!-- svelte-ignore a11y_autofocus -->
      <textarea placeholder="Reply" rows="2" maxlength="5000" required autofocus bind:value={replyBody}></textarea>
      <button type="submit">Add reply</button>
    </form>
  {/if}
</article>
