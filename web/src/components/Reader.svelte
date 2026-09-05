<script>
  // One document: the source beside it, the page itself, and everything said
  // about it.
  import { anchorAll, flatten } from "../lib/anchor.js";
  import * as sync from "../lib/sync.js";
  import * as renderers from "../lib/renderers.js";
  import * as collab from "../lib/collab.js";
  import { openRoom } from "../lib/room.js";
  import { getPrivate, me as whoami, postRaw, signInHref } from "../lib/api.js";
  import { AUTHOR, LINKED, markViewed, read, write } from "../lib/storage.js";
  import { PANES, clamp, rememberWidth, storedWidth } from "../lib/panes.js";

  import Nav from "./Nav.svelte";
  import IconButton from "./IconButton.svelte";
  import ControlGroup from "./ControlGroup.svelte";
  import CopyLink from "./CopyLink.svelte";
  import Modal from "./Modal.svelte";
  import Toasts from "./Toasts.svelte";
  import Row from "./layout/Row.svelte";
  import { problem as toastProblem } from "../lib/toast.svelte.js";
  import Preview from "./Preview.svelte";
  import Grip from "./Grip.svelte";
  import Sidebar from "./Sidebar.svelte";

  const SLUG = location.pathname.split("/").pop();

  /* ------------------------------------------------------------ the document */

  let doc = $state({});
  let docsOrigin = $state(null);
  let frameSrc = $state(null);
  let me = $state({});
  let identity = $derived(me.login || "");
  let canModerate = $derived(Boolean(doc.can_moderate));
  let connected = $state(true);

  // The version the frame is showing. A "published" broadcast naming this one
  // is our own save coming back, and means nothing new.
  let shownSHA = $state("");
  // The version this editor opened, sent back with every save. If the document
  // has moved on since -- someone else saved, or the same person in another
  // tab -- the server refuses rather than letting this save discard their work.
  let baseSHA = $state("");

  /* --------------------------------------------------------------- anchoring */

  let comments = $state([]);
  let commentsReady = false;
  let frameReady = false;
  let docText = null; // the joined visible text, invariant across repaints
  let docView = null; // flatten(docText), so anchoring does not redo it per call
  let figureAt = $state([]); // text offset of each figure, by its index

  let preview = $state(null);
  const tell = (message) => preview?.tell(message);

  // The agent repaints the whole document on every "regions" or "highlight"
  // message, so a call that changes nothing is not free even though it looks
  // idempotent. Each is sent only when its payload actually differs from the
  // last one sent -- reset when the frame republishes its text, since the
  // agent's DOM was rebuilt then and needs the full repaint regardless.
  let lastRegions = null;
  let lastHighlight = null;

  function applyHighlights() {
    if (!frameReady) return;
    const regions = JSON.stringify(
      comments
        .filter((comment) => comment.region)
        .map((comment) => ({
          id: comment.id,
          digest: comment.region.image_digest,
          index: comment.region.image_index,
          x: comment.region.x,
          y: comment.region.y,
          w: comment.region.w,
          h: comment.region.h,
          motivation: comment.motivation,
          resolved: Boolean(comment.resolved),
        })),
    );
    if (regions !== lastRegions) {
      lastRegions = regions;
      tell({ type: "regions", regions: JSON.parse(regions) });
    }

    const highlight = JSON.stringify(
      comments
        .filter((comment) => !comment.orphaned && comment.start != null)
        .map((comment) => ({
          id: comment.id,
          start: comment.start,
          end: comment.end,
          motivation: comment.motivation,
          resolved: Boolean(comment.resolved),
        })),
    );
    if (highlight !== lastHighlight) {
      lastHighlight = highlight;
      tell({ type: "highlight", ranges: JSON.parse(highlight) });
    }
  }

  function reanchor() {
    if (!frameReady || !commentsReady || docText === null) return;
    // A region annotation is placed by the agent, not by text matching, so it
    // is never orphaned for want of a quotation.
    anchorAll(docText, comments.filter((comment) => !comment.region), docView);
    comments = comments;
    applyHighlights();
  }

  function fromFrame(message) {
    switch (message.type) {
      case "ready":
        docText = typeof message.text === "string" ? message.text : "";
        docView = flatten(docText);
        // Where each figure sits in that text, so a note on a figure can be
        // ordered against the notes on passages.
        figureAt = Array.isArray(message.images) ? message.images.map(Number) : [];
        frameReady = true;
        // Whatever was painted before is gone with the rebuilt DOM.
        lastRegions = lastHighlight = null;
        reanchor();
        break;
      case "selection":
        showSelection(message.selector, message.rect);
        break;
      case "region":
        // A rectangle drawn on a figure anchors the same way a quotation does.
        pending = { exact: "", prefix: "", suffix: "", position: null, region: message.region };
        placeBar(message.rect);
        break;
      case "caret":
        followDocumentClick(Number(message.offset) || 0);
        break;
      case "focus":
        document.getElementById("comment-" + message.id)?.scrollIntoView({ behavior: "smooth", block: "center" });
        break;
    }
  }

  /* --------------------------------------------------------------- selection */

  let tool = $state("commenting");
  let pending = $state(null);
  let bar = $state({ shown: false, left: 0, top: 0 });

  function showSelection(selector, rect) {
    if (!selector || !selector.exact) {
      bar = { ...bar, shown: false };
      pending = null;
      return;
    }
    pending = {
      exact: String(selector.exact),
      prefix: String(selector.prefix || ""),
      suffix: String(selector.suffix || ""),
      // A hint, not a claim: the server keeps it, and anchoring uses it only
      // to choose between passages the context cannot separate.
      position: Number.isInteger(selector.position) && selector.position >= 0 ? selector.position : null,
    };
    placeBar(rect);
  }

  function placeBar(rect) {
    if (rect && !matchMedia("(max-width:760px)").matches) {
      const frameRect = document.querySelector(".viewport").getBoundingClientRect();
      bar = {
        shown: true,
        left: Math.min(innerWidth - 105, frameRect.left + rect.left + (rect.right - rect.left) / 2 - 42),
        top: Math.max(65, frameRect.top + rect.top - 42),
      };
      return;
    }
    bar = { ...bar, shown: true };
  }

  function chooseTool(which) {
    tool = which;
    tell({ type: "tool", tool: which });
  }

  /* -------------------------------------------------------------- annotating */

  let commenting = $state(false);
  let identifying = $state(false);
  let deleting = $state(false);
  let draft = $state({ body: "", tags: "", creator: read(AUTHOR, "Anonymous") });
  let pendingDelete = null;

  function barClicked() {
    if (!pending) return;
    bar = { ...bar, shown: false };
    if (tool === "highlighting") {
      // No dialog: the passage is the whole annotation.
      submitAnnotation({ motivation: "highlighting", body: "", tags: [] });
      return;
    }
    if (!identity && me.can_sign_in) {
      identifying = true;
      return;
    }
    commenting = true;
  }

  function submitAnnotation({ motivation, body, tags }) {
    if (!pending) return;
    const creator = identity || draft.creator;
    if (!identity) write(AUTHOR, creator);
    const temp_id = crypto.randomUUID();
    const optimistic = {
      id: temp_id,
      temp_id,
      seq: Number.MAX_SAFE_INTEGER,
      ...pending,
      motivation,
      body,
      tags,
      creator: creator || "Anonymous",
      created: new Date().toISOString(),
      resolved: false,
      resolved_at: null,
      replies: [],
      pending: true,
    };
    // Drawn before the round trip; the broadcast reconciles it by temp_id.
    anchorAll(docText || "", [optimistic], docText === null ? null : docView);
    comments = [...comments, optimistic];
    applyHighlights();
    room?.send({ type: "comment", ...pending, motivation, body, tags, creator, temp_id });
    pending = null;
  }

  function submitDialog(event) {
    event.preventDefault();
    submitAnnotation({
      motivation: tool === "region" ? "commenting" : tool,
      body: draft.body,
      // "methods, typo" becomes ["methods", "typo"]. The server normalises
      // again, so this only has to be reasonable.
      tags: draft.tags.split(",").map((tag) => tag.trim()).filter(Boolean),
    });
    draft = { ...draft, body: "", tags: "" };
    commenting = false;
  }

  function resolve(comment) {
    // Optimistic: flip locally, then tell the room. The broadcast that comes
    // back is idempotent with what we already drew.
    comment.resolved = !comment.resolved;
    comments = comments;
    applyHighlights();
    room?.send({ type: "resolve", comment_id: comment.id, resolved: comment.resolved });
  }

  function askDelete(comment) {
    pendingDelete = comment;
    deleting = true;
  }

  function confirmDelete() {
    const comment = pendingDelete;
    pendingDelete = null;
    deleting = false;
    if (!comment) return;
    comments = comments.filter((item) => item !== comment);
    applyHighlights();
    room?.send({ type: "delete", comment_id: comment.id });
  }

  function reply(comment, body, name) {
    const temp_id = crypto.randomUUID();
    comment.replies = [
      ...comment.replies,
      { id: temp_id, body, creator: name || "Anonymous", created: new Date().toISOString(), temp_id },
    ];
    comments = comments;
    room?.send({ type: "reply", comment_id: comment.id, body, creator: name, temp_id });
  }

  /* -------------------------------------------------------------------- room */

  let room = null;

  function receive(event) {
    if (event.type === "hello") {
      comments = event.comments;
      commentsReady = true;
      reanchor();
      return;
    }
    if (event.type === "error") {
      // Roll the optimistic row back.
      if (event.temp_id) {
        comments = comments
          .filter((comment) => comment.temp_id !== event.temp_id)
          .map((comment) => ({
            ...comment,
            replies: comment.replies.filter((reply) => reply.temp_id !== event.temp_id),
          }));
        applyHighlights();
      }
      // A refused delete or resolve was applied optimistically before the
      // server had a say; the list is re-fetched so the optimistic change goes
      // back out.
      if (event.comment_id) {
        fetch(`/api/documents/${SLUG}/comments`)
          .then((response) => response.json())
          .then((data) => receive({ type: "hello", comments: data.comments }))
          .catch(() => {});
      }
      toastProblem(event.message);
      return;
    }

    // The shared source: the state of a session as it stands, one more change
    // to it, who else is in it, or where their carets are. All of it is
    // meaningless unless this browser is editing, and ignored until it is.
    if (event.type === "y-state") {
      session?.start(event, savedSource);
      peers = event.count || 1;
      return;
    }
    if (event.type === "y-update") {
      session?.apply(event.update);
      return;
    }
    if (event.type === "y-awareness") {
      session?.applyAwareness(event.update);
      return;
    }
    if (event.type === "y-snapshot") {
      // The session's history grew long enough that a latecomer would have to
      // replay all of it. This browser sends the whole state instead.
      if (session) room.send({ type: "y-update", update: session.snapshot(), replace: true });
      return;
    }
    if (event.type === "y-peers") {
      peers = event.count || 1;
      return;
    }

    if (event.type === "published") {
      published(event);
      return;
    }

    if (event.type === "comment") {
      const local = comments.find((comment) => comment.temp_id === event.temp_id);
      // Broadcasts carry no `deletable` field, so the caller's own comment,
      // reconciled here from its optimistic placeholder, stays deletable by
      // this browser regardless of what the server sent back.
      if (local) Object.assign(local, event.comment, { temp_id: undefined, pending: false, deletable: true });
      else if (!comments.some((comment) => comment.id === event.comment.id)) {
        anchorAll(docText || "", [event.comment], docText === null ? null : docView);
        comments = [...comments, event.comment];
      }
      comments = comments;
      applyHighlights();
      return;
    }
    if (event.type === "reply") {
      const comment = comments.find((item) => item.id === event.comment_id);
      if (!comment) return;
      const local = comment.replies.find((reply) => reply.temp_id === event.temp_id);
      if (local) Object.assign(local, event.reply, { temp_id: undefined });
      else if (!comment.replies.some((reply) => reply.id === event.reply.id)) {
        comment.replies = [...comment.replies, event.reply];
      }
      comments = comments;
      return;
    }
    if (event.type === "delete") {
      comments = comments.filter((item) => item.id !== event.comment_id);
      applyHighlights();
      return;
    }
    if (event.type === "resolve") {
      const comment = comments.find((item) => item.id === event.comment_id);
      if (!comment) return;
      comment.resolved = event.resolved;
      comment.resolved_at = event.resolved_at;
      comments = comments;
      applyHighlights();
    }
  }

  /* ----------------------------------------------------------------- editing */

  // CodeMirror and Yjs are a third of a megabyte, and most people who open a
  // document are here to read it. The editor is fetched when one is actually
  // opened, so a reader never pays for it.
  let Editor = $state(null);
  let editor = $state(null);
  let session = $state(null);
  let editing = $state(false);
  let sourceFormat = $state("");
  let savedSource = $state("");
  let state = $state(""); // what the editor is saying about itself
  let problem = $state(false);
  let peers = $state(1);
  let linked = $state(read(LINKED, false) === true);

  const dirty = $derived(editing && session && session.text.toString() !== savedSource);

  function say(text, isProblem = false) {
    state = text;
    problem = isProblem;
  }

  // What the document is called, which is what the rendered page is titled.
  // The title it was published under wins; a document that never had one is
  // named by its own first heading, the way `publish` names one.
  async function headingOf(source) {
    return doc.title || (await renderers.titleOf(source, sourceFormat)) || "Untitled";
  }

  // Painting the preview is sending it to the frame: the draft is a document,
  // and a document belongs on the documents origin, not in this page. The
  // agent republishes its text from there, which re-anchors every comment
  // against what was just typed.
  let issued = 0;
  let painted = 0;
  let previewTimer = null;

  async function paintPreview() {
    const mine = ++issued;
    const source = session ? session.text.toString() : "";
    try {
      const html = await renderers.render(source, await headingOf(source), sourceFormat);
      // A slower render that resolves late must not paint over a newer one.
      if (mine <= painted) return;
      painted = mine;
      tell({ type: "preview", html });
      say(dirty ? "unsaved changes" : "saved");
    } catch (error) {
      if (mine > painted) say(error.message || "could not render", true);
    }
  }

  // Short enough to read as live -- the renderer takes single-digit
  // milliseconds -- and long enough that a burst of typing is one render.
  function sourceChanged() {
    say(dirty ? "unsaved changes" : "saved");
    clearTimeout(previewTimer);
    previewTimer = setTimeout(paintPreview, 60);
  }

  /* ------------------------------------------------------- keeping in step */

  // Said when the lock has nowhere to go: the words at the caret, and the words
  // around them, are in neither the document nor the source. That is rare now
  // that it looks beside the line as well as at it -- a formula, a blank line
  // and a fenced block all resolve to the prose next to them -- so when it does
  // happen it is worth one plain line rather than an alarm. Nothing is broken:
  // the lock is on and the next move will try again.
  const NO_MATCH = "nothing to jump to here";

  function lost(yes) {
    if (!yes) {
      if (state === NO_MATCH) say(dirty ? "unsaved changes" : "saved");
      return;
    }
    // Not a problem: the editor is in the state it was in, and the reader has
    // lost nothing. It is a fact about where the caret happens to be.
    say(NO_MATCH);
  }

  let stepTimer = null;
  function followCaret() {
    if (!linked || !editing || docText === null) return;
    clearTimeout(stepTimer);
    stepTimer = setTimeout(() => {
      const place = sync.documentPlaceFor(editor.text(), editor.caret(), docText);
      if (place) {
        tell({ type: "locate", start: place.at, length: place.length });
        lost(false);
        return;
      }
      // The words at the caret are not findable in the document: a formula, a
      // table cell, a heading that renders as something else. Said rather than
      // ignored, because a lock that silently does nothing is
      // indistinguishable from one that is broken.
      lost(true);
    }, 120);
  }

  function followDocumentClick(offset) {
    if (!linked || !editing || docText === null || !editor) return;
    const at = sync.sourcePlaceFor(docText, offset, editor.text());
    if (at === null) {
      lost(true);
      return;
    }
    lost(false);
    editor.goTo(at);
  }

  function setLinked(on) {
    linked = on;
    write(LINKED, on);
  }

  /* ------------------------------------------------------------------ saving */

  async function save() {
    if (!dirty) return;
    say("saving…");
    const source = session.text.toString();
    try {
      const title = await headingOf(source);
      const html = await renderers.render(source, title, sourceFormat);
      const response = await postRaw("/api/documents", {
        title, slug: SLUG, html, source, source_format: sourceFormat, base_sha: baseSHA,
      });
      const body = await response.json().catch(() => ({}));
      if (!response.ok) {
        // A conflict is the one refusal worth spelling out: nothing was
        // written, and what to do about it is the reader's decision.
        say(
          response.status === 409
            ? "published elsewhere while you were editing — reload to see it before saving over it"
            : body.error || `save failed (${response.status})`,
          true,
        );
        return;
      }
      savedSource = source;
      baseSHA = shownSHA = body.sha || baseSHA;
      say("saved");
    } catch (error) {
      say(error.message || "save failed", true);
    }
  }

  // Someone published a new version of this document -- another reader saving
  // in their own editor, or the same document republished from the command
  // line. What to do about it depends on what this reader is in the middle of.
  async function published(event) {
    if (!event.sha || event.sha === shownSHA) return;
    // In a shared session everyone is typing into one source, so a save by any
    // of them publishes what all of them have. That is not a conflict, and
    // warning about it would make collaborating feel like colliding.
    if (session) {
      shownSHA = event.sha;
      const payload = await getPrivate(`/api/documents/${SLUG}/source`).catch(() => null);
      if (!payload) return;
      baseSHA = payload.sha || "";
      if ((payload.source || "") === session.text.toString()) {
        savedSource = payload.source || "";
        say("saved");
        return;
      }
      // Published from outside the session -- the command line, or a browser
      // that was not in it. Their work is not this page's to discard, and the
      // save that would discard it is refused anyway.
      say("a newer version was published — reload before saving", true);
      return;
    }
    if (dirty) {
      say("a newer version was published — reload before saving", true);
      return;
    }
    shownSHA = event.sha;
    // Reading rather than editing: show the version that now exists. The frame
    // republishes its text on load, which re-anchors every comment.
    frameSrc = `${docsOrigin}/raw/${SLUG}/${event.sha}.html`;
  }

  /* ------------------------------------------------------------------ panes */

  let showing = $state({ source: false, preview: true, comments: true });
  let widths = $state({ [PANES.editor.key]: storedWidth(PANES.editor), [PANES.sidebar.key]: storedWidth(PANES.sidebar) });
  let guide = $state({ shown: false, left: 0 });
  let grabbing = $state(false);

  const separators = () => (showing.source ? 8 : 0) + (showing.comments ? 8 : 0);

  function fit() {
    for (const pane of Object.values(PANES)) {
      widths[pane.key] = clamp(pane, widths[pane.key], { widths, showing, separators: separators() });
    }
  }

  function setWidth(pane, width, step) {
    const wanted = width === null ? widths[pane.key] + step : width;
    widths[pane.key] = clamp(pane, wanted, { widths, showing, separators: separators() });
    rememberWidth(pane, widths[pane.key]);
  }

  // Hiding the last pane would leave the window empty, which is never what was
  // meant: the button refuses, and stays pressed.
  function togglePane(which) {
    const on = showing[which];
    if (on && Object.values(showing).filter(Boolean).length === 1) return;
    showing[which] = !on;
    fit();
  }

  /* ------------------------------------------------------------------- boot */

  // Editing a document and looking at its source are two different things. The
  // session lasts as long as the document is open; the pane is a view of it,
  // which can be folded away like any other.
  function startEditing() {
    if (editing) return;
    editing = true;
    say("saved");
    session = collab.join({
      send: (message) => room.send(message),
      onPeers: (count) => (peers = Math.max(peers, count)),
      name: identity || read(AUTHOR, "Anonymous"),
    });
    session.text.observe(sourceChanged);
    room.send({ type: "y-open" });
    showing.source = true;
    fit();
    paintPreview();
  }

  // Offered only when there is something to edit and someone allowed to edit
  // it. The source is asked for once, and the renderer starts downloading with
  // it, so opening the editor does not then wait for the module.
  async function offerEditing(document_) {
    // can_edit, not can_moderate: moderating is about this document's
    // comments, editing is about replacing the document.
    const mayEdit = document_.can_edit === undefined ? document_.can_moderate : document_.can_edit;
    if (!mayEdit || !document_.source_format) return;
    // A document is editable here only if this deployment can render what it
    // was written in. Markdown always; typst when its renderer was built.
    const list = Array.isArray(document_.renderers) ? document_.renderers : ["markdown"];
    if (!list.includes(document_.source_format) || !renderers.available(document_.source_format)) {
      say(`${document_.source_format} documents are edited where their renderer is built`);
      return;
    }
    sourceFormat = document_.source_format;
    Editor = (await import("./Editor.svelte")).default;
    const payload = await getPrivate(`/api/documents/${SLUG}/source`).catch(() => null);
    if (!payload) return; // not editable here; the reader is unchanged
    savedSource = payload.source || "";
    baseSHA = payload.sha || "";
    renderers.warm(sourceFormat);
    // A document with a source opens ready to be worked on: that is what its
    // author came for.
    startEditing();
  }

  $effect(() => {
    markViewed(SLUG);
    room = openRoom(SLUG, { onMessage: receive, onConnected: (up) => (connected = up) });

    whoami().then((who) => {
      me = who;
      if (who.login) {
        draft.creator = who.login;
        session?.rename(who.login);
      }
    });

    fetch(`/api/documents/${SLUG}`)
      .then((response) => (response.ok ? response.json() : Promise.reject(new Error("not found"))))
      .then((found) => {
        doc = found;
        document.title = `${found.title} · Komodoc`;
        docsOrigin = found.docs_origin || location.origin;
        shownSHA = found.sha;
        frameSrc = `${docsOrigin}/raw/${SLUG}/${found.sha}.html`;
        offerEditing(found);
      })
      .catch(() => (doc = { title: "Document not found" }));

    return () => {
      // The session on the server ends when the last person in it
      // disconnects, which the socket closing does on its own; this is only
      // this browser letting go of its half.
      session?.leave();
      room?.close();
    };
  });

  // A tab closed mid-edit is work lost, so the browser asks first.
  function beforeUnload(event) {
    if (dirty) event.preventDefault();
  }
</script>

<svelte:window onresize={fit} onbeforeunload={beforeUnload} onpagehide={() => session?.leave()} />

<Nav {me}>
  {#snippet children()}
    <span id="docTitle" class="text-surface-600-400 truncate text-sm">{doc.title ?? ""}</span>
    <!-- Silent while the socket is up: it only has something to say when the
         live updates have stopped. -->
    {#if !connected}
      <small class="badge preset-tonal-warning whitespace-nowrap">reconnecting…</small>
    {/if}
  {/snippet}

  {#snippet tools()}
    <Row gap={3}>
      <!-- Which panes are showing. A group of its own, separate from the
           annotation tools: these change what you are looking at, not what a
           selection would become. -->
      <ControlGroup label="Panes">
        {#snippet children()}
          {#if editing}
            <IconButton icon="panel-left" label="Show or hide the source" title="Source"
                        pressed={showing.source} onclick={() => togglePane("source")} />
          {/if}
          <IconButton icon="file-text" label="Show or hide the document" title="Document"
                      pressed={showing.preview} onclick={() => togglePane("preview")} />
          <IconButton icon="panel-right" label="Show or hide the comments" title="Comments"
                      pressed={showing.comments} onclick={() => togglePane("comments")} />
          {#if editing}
            <!-- Locked, a click in either the source or the document takes the
                 other one to the same place. -->
            <IconButton icon="lock" label="Keep the source and the document in step"
                        title="Keep in step" pressed={linked} onclick={() => setLinked(!linked)} />
          {/if}
        {/snippet}
      </ControlGroup>
      {#if editing}
        <IconButton icon="save" label="Save a new version" title="Save"
                    tone="primary" disabled={!dirty} onclick={save} />
        <!-- Said only when there is more than one person editing. -->
        {#if peers > 1}
          <small class="badge preset-tonal-secondary whitespace-nowrap">{peers} editing</small>
        {/if}
        {#if state}
          <small class="badge whitespace-nowrap {problem ? 'preset-tonal-error' : 'preset-tonal-surface'}">
            {state}
          </small>
        {/if}
      {/if}
      <CopyLink label="Copy the link to this document" />
    </Row>
  {/snippet}
</Nav>

<main class="reader" class:editing={showing.source} class:no-preview={!showing.preview}
      class:no-comments={!showing.comments}
      style="--komodoc-editor: {widths[PANES.editor.key]}px; --komodoc-sidebar: {widths[PANES.sidebar.key]}px">
  {#if showing.source}
    <section class="editorpane">
      {#if Editor}
        <Editor bind:this={editor} {session} format={sourceFormat}
                onchange={sourceChanged} oncaret={followCaret} onsave={save} />
      {/if}
    </section>
    <Grip pane={PANES.editor} label="Resize the editor pane"
          onwidth={(width, step) => setWidth(PANES.editor, width ?? null, step)}
          onguide={(width) => (guide = { shown: true, left: PANES.editor.edgeAt(clamp(PANES.editor, width, { widths, showing, separators: separators() })) })}
          ongrab={(on) => { grabbing = on; guide = { ...guide, shown: on }; }} />
  {/if}

  <Preview bind:this={preview} src={frameSrc} {docsOrigin} onmessage={fromFrame} {grabbing} />

  {#if showing.comments}
    <Grip pane={PANES.sidebar} label="Resize the comment pane"
          onwidth={(width, step) => setWidth(PANES.sidebar, width ?? null, step)}
          onguide={(width) => (guide = { shown: true, left: PANES.sidebar.edgeAt(clamp(PANES.sidebar, width, { widths, showing, separators: separators() })) })}
          ongrab={(on) => { grabbing = on; guide = { ...guide, shown: on }; }} />
    <Sidebar {comments} {figureAt} {identity} {canModerate} {tool}
             hasFigures={figureAt.length > 0}
             ontool={chooseTool}
             onreveal={(comment) => tell({ type: "reveal", id: comment.id })}
             onresolve={resolve} ondelete={askDelete} onreply={reply} />
  {/if}

  <!-- Shown only while a separator is dragged: a line that follows the pointer
       so the split can be seen moving without the iframe reflowing on every
       pointermove. -->
  {#if guide.shown}<div class="grip-guide" style="left: {guide.left}px"></div>{/if}
</main>

{#if bar.shown}
  <button
    id="selectionbar"
    class="btn btn-sm preset-filled-primary-500 shadow-lg"
    style="display: block; left: {bar.left}px; top: {bar.top}px"
    onclick={barClicked}
  >
    {tool === "highlighting" ? "Highlight" : tool === "region" ? "Box" : "Comment"}
  </button>
{/if}

<!-- What a selection becomes, once the reader has said what to call it and
     what they think of it. -->
<Modal bind:open={commenting} title="Add comment">
  {#snippet children()}
    <form id="commentForm" class="flex flex-col gap-3" onsubmit={submitDialog}>
      <blockquote class="border-primary-500 text-surface-700-300 border-l-2 pl-3 text-sm">
        {pending?.region ? `Figure ${pending.region.image_index + 1}` : `“${pending?.exact ?? ""}”`}
      </blockquote>
      {#if identity}
        <p class="text-surface-600-400 text-sm">commenting as @{identity}</p>
      {:else if me.comments_need_login}
        <p class="text-sm">
          <a class="anchor" href={signInHref()}>Sign in with GitHub</a> to comment on this document.
        </p>
      {:else}
        <label class="label">
          <span class="label-text">Name</span>
          <input class="input" maxlength="80" bind:value={draft.creator} />
        </label>
      {/if}
      <label class="label">
        <span class="label-text">Comment</span>
        <!-- svelte-ignore a11y_autofocus -->
        <textarea class="textarea" rows="5" maxlength="5000" required autofocus bind:value={draft.body}
        ></textarea>
      </label>
      <label class="label">
        <span class="label-text">Tags <small class="text-surface-500">optional, comma separated</small></span>
        <input class="input" placeholder="methods, typo, citation" bind:value={draft.tags} />
      </label>
    </form>
  {/snippet}
  {#snippet footer()}
    <button type="button" class="btn preset-outlined-surface-300-700" onclick={() => (commenting = false)}>
      Cancel
    </button>
    <button
      type="submit"
      form="commentForm"
      class="btn preset-filled-primary-500"
      disabled={!identity && me.comments_need_login}
    >
      Save
    </button>
  {/snippet}
</Modal>

<!-- Deleting a thread cannot be undone, so it is confirmed. -->
<Modal
  bind:open={deleting}
  title="Delete comment?"
  description="This removes the comment and its replies for everyone. It cannot be undone."
>
  {#snippet footer()}
    <button type="button" class="btn preset-outlined-surface-300-700" onclick={() => (deleting = false)}>
      Cancel
    </button>
    <button type="button" class="btn preset-filled-error-500" onclick={confirmDelete}>Delete</button>
  {/snippet}
</Modal>

<!-- Signed out, a commenter chooses how to be named before they write. -->
<Modal
  bind:open={identifying}
  title="Who are you?"
  description="Choose how to identify yourself in this comment."
>
  {#snippet footer()}
    <button
      type="button"
      class="btn preset-outlined-surface-300-700"
      onclick={() => { identifying = false; location.href = signInHref(); }}
    >
      Sign in with GitHub
    </button>
    <button
      type="button"
      class="btn preset-filled-primary-500"
      onclick={() => { identifying = false; commenting = true; }}
    >
      Enter a name
    </button>
  {/snippet}
</Modal>

<Toasts />
