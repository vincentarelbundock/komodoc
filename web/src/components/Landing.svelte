<script>
  // The landing page: what you may publish, and what you have published.
  import Nav from "./Nav.svelte";
  import CopyLink from "./CopyLink.svelte";
  import { SHELL_HEADERS, config as loadConfig, get, me as whoami, upload } from "../lib/api.js";
  import { FAVORITES, VIEWED, read, write } from "../lib/storage.js";

  let me = $state({});
  let config = $state({ max_html: 4 * 1024 * 1024, extensions: [".html", ".htm", ".md", ".markdown"] });
  let documents = $state([]);
  let counts = $state(new Map());
  let favorites = $state(new Set(read(FAVORITES, [])));
  let viewed = $state(read(VIEWED, {}));
  let selected = $state(new Set());
  let search = $state("");
  let tab = $state("all");
  let sortBy = $state("updated");
  let ascending = $state(false);
  let fileError = $state("");
  let chosen = $state(null);
  let title = $state("");
  let busy = $state(false);
  let dragging = $state(false);
  let shared = $state(null);
  let confirmDialog = $state(null);
  let confirmText = $state("");
  let sharedDialog = $state(null);
  let fileInput = $state(null);
  let titleInput = $state(null);

  const maxLabel = $derived(Math.round(config.max_html / (1024 * 1024)) + " MB");

  /* ------------------------------------------------------------- the listing */

  // "3 days ago" rather than a date: for something you did yourself, how long
  // ago is the question, not when.
  function sinceWhen(stamp) {
    const days = Math.floor((Date.now() - new Date(stamp)) / 86400000);
    if (days <= 0) return "today";
    if (days === 1) return "yesterday";
    if (days < 30) return `${days} days ago`;
    return new Date(stamp).toISOString().slice(0, 10);
  }

  // One comparison per column. Documents never opened sort as if they were
  // opened at the beginning of time, so they gather at one end rather than
  // scattering through the list.
  function comparing(column, up) {
    const key = {
      title: (doc) => doc.title.toLowerCase(),
      comments: (doc) => counts.get(doc.slug) ?? -1,
      updated: (doc) => doc.updated_at,
      viewed: (doc) => viewed[doc.slug] || "",
    }[column];
    return (a, b) => {
      const left = key(a);
      const right = key(b);
      const order = left < right ? -1 : left > right ? 1 : 0;
      return up ? order : -order;
    };
  }

  const shown = $derived.by(() => {
    const needle = search.trim().toLowerCase();
    return documents
      .filter(
        (doc) =>
          (tab === "all" || favorites.has(doc.slug)) &&
          (!needle || doc.title.toLowerCase().includes(needle)),
      )
      .sort(comparing(sortBy, ascending));
  });

  const hereSelected = $derived(shown.filter((doc) => selected.has(doc.slug)).length);

  function sortColumn(column) {
    // Clicking the column already sorted reverses it; a new column starts in
    // the order that column is usually wanted in.
    if (sortBy === column) ascending = !ascending;
    else {
      sortBy = column;
      ascending = column === "title";
    }
  }

  function star(slug) {
    const next = new Set(favorites);
    next.has(slug) ? next.delete(slug) : next.add(slug);
    favorites = next;
    write(FAVORITES, [...next]);
  }

  function tick(slug, on) {
    const next = new Set(selected);
    on ? next.add(slug) : next.delete(slug);
    selected = next;
  }

  function tickAll(on) {
    const next = new Set(selected);
    for (const doc of shown) (on ? next.add(doc.slug) : next.delete(doc.slug));
    selected = next;
  }

  async function showList() {
    const listing = await fetch("/api/list", { method: "POST", headers: SHELL_HEADERS });
    if (!listing.ok) return;
    documents = (await listing.json()).documents;
    // Counts live in each document's room rather than in the index, so they
    // are fetched separately and the list picks them up when they land.
    const found = new Map();
    await Promise.all(
      documents.map((doc) =>
        get(`/api/documents/${doc.slug}`)
          .then((full) => found.set(doc.slug, full.comment_count))
          .catch(() => {}),
      ),
    );
    counts = found;
  }

  async function deleteSelected() {
    const slugs = [...selected];
    if (!slugs.length) return;
    const plural = slugs.length === 1 ? "" : "s";
    confirmText = `This permanently removes ${slugs.length} document${plural} and every comment on ${slugs.length === 1 ? "it" : "them"}. It cannot be undone.`;
    confirmDialog.returnValue = "";
    confirmDialog.showModal();
    const ok = await new Promise((resolve) =>
      confirmDialog.addEventListener("close", () => resolve(confirmDialog.returnValue === "ok"), { once: true }),
    );
    if (!ok) return;
    await Promise.all(
      slugs.map((slug) =>
        fetch(`/api/documents/${slug}/delete`, { method: "POST", headers: SHELL_HEADERS }).catch(() => {}),
      ),
    );
    const keep = new Set(favorites);
    for (const slug of slugs) keep.delete(slug);
    favorites = keep;
    write(FAVORITES, [...keep]);
    selected = new Set();
    await showList();
  }

  /* ------------------------------------------------------------ file picking */

  function refuse(message) {
    chosen = null;
    title = "";
    fileError = message;
    return false;
  }

  // Checked here so a 30 MB mistake is caught before the upload, not after.
  function valid(file) {
    if (!file) return false;
    const dot = file.name.lastIndexOf(".");
    const extension = dot < 0 ? "" : file.name.slice(dot).toLowerCase();
    if (!config.extensions.includes(extension)) {
      return refuse(`${file.name} is not a document Komodoc can serve. Only ${config.extensions.join(", ")} work.`);
    }
    if (file.size > config.max_html) {
      return refuse(`${file.name} is ${(file.size / (1024 * 1024)).toFixed(1)} MB; the limit is ${maxLabel}.`);
    }
    return true;
  }

  // The document usually names itself: <title> or the first heading in HTML,
  // the first setext or ATX H1 in Markdown. The filename is the fallback, and
  // the CLI does the same.
  function titleFrom(text, name) {
    if (/\.html?$/i.test(name)) {
      const parsed = new DOMParser().parseFromString(text, "text/html");
      const found = parsed.title.trim() || parsed.querySelector("h1")?.textContent.trim();
      if (found) return found;
    } else {
      const atx = text.match(/^#\s+(.+)$/m);
      if (atx) return atx[1].trim();
      const setext = text.match(/^(.+)\n=+\s*$/m);
      if (setext) return setext[1].trim();
    }
    return name.replace(/\.[^.]+$/, "").replace(/[_-]+/g, " ").trim();
  }

  // The drop zone has done its job, so it gives way to the one remaining
  // decision. The title arrives selected, so typing replaces it and Enter
  // alone accepts it.
  async function choose(file) {
    if (!valid(file)) return;
    fileError = "";
    chosen = file;
    title = file.name;
    await Promise.resolve();
    titleInput?.select();
    const text = await file.text().catch(() => "");
    // The reader may already be typing by the time the file is read.
    if (title === file.name) {
      title = titleFrom(text, file.name);
      titleInput?.select();
    }
  }

  function drop(event) {
    event.preventDefault();
    dragging = false;
    const file = event.dataTransfer?.files?.[0];
    if (file) choose(file);
  }

  async function submit(event) {
    event.preventDefault();
    if (!chosen) {
      refuse("Choose a document to upload.");
      return;
    }
    busy = true;
    const form = new FormData();
    form.append("file", chosen, chosen.name);
    form.append("title", title);
    const response = await upload(form);
    busy = false;
    if (!response.ok) {
      alert((await response.json().catch(() => ({}))).error || "upload failed");
      return;
    }
    const doc = await response.json();
    chosen = null;
    title = "";
    await showList();
    shared = new URL(doc.url, location.origin).href;
    sharedDialog.showModal();
  }

  $effect(() => {
    loadConfig().then((found) => (config = found)).catch(() => {});
    whoami().then(async (who) => {
      me = who;
      if (who.can_publish) await showList();
    });
  });
</script>

<svelte:window ondragover={(event) => event.preventDefault()} ondrop={drop} />

<Nav {me} />

<main class="container">
  <hgroup>
    <h1>Komodoc</h1>
    <p>Publish a document, share its link, and collect comments on it.</p>
  </hgroup>

  {#if me.login && !me.can_publish}
    <p class="refused">@{me.login} may not publish here; this deployment allows {me.publishers}.</p>
  {/if}

  {#if me.can_publish}
    <form id="uploadForm" onsubmit={submit}>
      {#if !chosen}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="dropzone" class:dragging
             ondragenter={(event) => { event.preventDefault(); dragging = true; }}
             ondragover={(event) => { event.preventDefault(); dragging = true; }}
             ondragleave={() => (dragging = false)}
             ondrop={drop}>
          <p>Drop a document here</p>
          <button type="button" onclick={() => fileInput.click()}>Choose a file</button>
          <small>{config.extensions.join(" or ")} files, up to {maxLabel}</small>
          <input type="file" bind:this={fileInput} hidden
                 accept={config.extensions.join(",") + ",text/html"}
                 onchange={(event) => event.currentTarget.files[0] && choose(event.currentTarget.files[0])} />
        </div>
      {:else}
        <div class="chosen">
          <p><strong>{chosen.name}</strong></p>
          <label for="title">Title</label>
          <!-- Escape backs out of the choice rather than only clearing the
               field. -->
          <input id="title" name="title" bind:this={titleInput} bind:value={title}
                 onkeydown={(event) => { if (event.key === "Escape") { event.preventDefault(); chosen = null; } }} />
          <div class="grid">
            <button type="button" class="secondary" onclick={() => (chosen = null)}>Cancel</button>
            <button type="submit" disabled={busy}>{busy ? "Adding…" : "Add document"}</button>
          </div>
        </div>
      {/if}
      {#if fileError}<p class="fileerror">{fileError}</p>{/if}
    </form>
  {/if}

  {#if documents.length || me.can_publish}
    <div class="browse">
      <div role="group">
        <button type="button" aria-pressed={tab === "all"} onclick={() => (tab = "all")}>All</button>
        <button type="button" aria-pressed={tab === "favorites"} onclick={() => (tab = "favorites")}>Favorites</button>
      </div>
      <input type="search" placeholder="Search titles" bind:value={search} />
    </div>

    {#if selected.size}
      <div class="selection">
        <span>{selected.size} selected</span>
        <button type="button" onclick={deleteSelected}>Delete</button>
      </div>
    {/if}

    <table>
      <thead>
        <tr>
          <th>
            <input type="checkbox" aria-label="Select every document shown"
                   checked={shown.length > 0 && hereSelected === shown.length}
                   indeterminate={hereSelected > 0 && hereSelected < shown.length}
                   onchange={(event) => tickAll(event.currentTarget.checked)} />
          </th>
          <th></th>
          <th><button type="button" class="sort" class:active={sortBy === "title"} onclick={() => sortColumn("title")}>Title</button></th>
          <th><button type="button" class="sort" class:active={sortBy === "comments"} onclick={() => sortColumn("comments")}>Comments</button></th>
          <th><button type="button" class="sort" class:active={sortBy === "updated"} onclick={() => sortColumn("updated")}>Updated</button></th>
          <th><button type="button" class="sort" class:active={sortBy === "viewed"} onclick={() => sortColumn("viewed")}>Opened</button></th>
        </tr>
      </thead>
      <tbody>
        {#each shown as doc (doc.slug)}
          <tr>
            <td>
              <input type="checkbox" aria-label="Select {doc.title}" checked={selected.has(doc.slug)}
                     onchange={(event) => tick(doc.slug, event.currentTarget.checked)} />
            </td>
            <td>
              <button type="button" class="star" aria-pressed={favorites.has(doc.slug)}
                      aria-label={favorites.has(doc.slug) ? "Remove from favorites" : "Add to favorites"}
                      onclick={() => star(doc.slug)}>{favorites.has(doc.slug) ? "★" : "☆"}</button>
            </td>
            <td class="titlecell">
              <a href="/docs/{doc.slug}">{doc.title}</a>
              <!-- What the document is written in. A document that kept its
                   source opens in the editor rather than only in the reader. -->
              <small class="kind" title={doc.source_format
                ? `Published from ${doc.source_format}, and editable`
                : "HTML: published as it is, and read-only here"}>
                {({ markdown: ".md", typst: ".typ" })[doc.source_format] || ".html"}
              </small>
              <CopyLink href={new URL(`/docs/${doc.slug}`, location.origin).href}
                        label="Copy the link to {doc.title}" />
            </td>
            <td>{counts.has(doc.slug) ? counts.get(doc.slug) : "—"}</td>
            <td>{doc.updated_at.slice(0, 10)}</td>
            <td class:never={!viewed[doc.slug]}>{viewed[doc.slug] ? sinceWhen(viewed[doc.slug]) : "never"}</td>
          </tr>
        {:else}
          <tr>
            <td colspan="6" class="emptyrow">
              {documents.length === 0
                ? "No documents uploaded yet."
                : tab === "favorites" && !search.trim()
                  ? "No favorites yet. Star a document to keep it here."
                  : "No documents match that search."}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</main>

<dialog bind:this={confirmDialog}>
  <article>
    <form method="dialog">
      <h3>Delete?</h3>
      <p>{confirmText}</p>
      <footer class="grid">
        <button type="submit" value="cancel" class="secondary">Cancel</button>
        <button type="submit" value="ok" autofocus>OK</button>
      </footer>
    </form>
  </article>
</dialog>

<dialog bind:this={sharedDialog}>
  <article>
    <h3>Published</h3>
    <p>Share this link; anyone with it can comment, no account needed.</p>
    <input readonly value={shared ?? ""} />
    <footer class="grid">
      <a role="button" class="secondary" href={shared ?? "/"}>Open</a>
      <button type="button" onclick={() => sharedDialog.close()}>Done</button>
    </footer>
  </article>
</dialog>
