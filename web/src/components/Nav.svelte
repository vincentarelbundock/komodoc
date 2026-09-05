<script>
  import Logo from "./Logo.svelte";
  import Icon from "./Icon.svelte";
  import Row from "./layout/Row.svelte";
  import { signInHref, signOut } from "../lib/api.js";

  // The bar every page wears: the mark, whatever the page puts in the middle,
  // and who you are.
  //
  // One row, centred, with a gap: the vertical rhythm is decided here and
  // nowhere else, so a control added later cannot land half a line above its
  // neighbours.
  let { me = {}, children, tools } = $props();
</script>

<nav class="flex items-center gap-4">
  <Row gap={3}>
    <a class="flex items-center gap-2" href="/" aria-label="Komodoc home">
      <Logo />
      <strong class="wordmark"><span class="wordmark-komo">komo</span><span class="wordmark-doc">doc</span></strong>
    </a>
    {@render children?.()}
  </Row>

  <!-- The one link that is the same on every page: what Komodoc is and how to
       use it, from the project's own README. -->
  <a
    href="/documentation"
    class="text-surface-600-400 hover:text-primary-500 mx-auto flex items-center gap-2 text-sm"
    aria-label="Documentation"
    title="Documentation"
  >
    <Icon name="book" size={17} />
    <span class="navlabel">Documentation</span>
  </a>

  <Row gap={3}>
    {@render tools?.()}
    {#if me.login}
      <small class="text-surface-600-400 whitespace-nowrap">@{me.login}</small>
      <button type="button" class="btn btn-sm preset-outlined-surface-300-700" onclick={signOut}>
        Sign out
      </button>
    {:else if me.can_sign_in}
      <a role="button" class="btn btn-sm preset-filled-primary-500" href={signInHref()}>Sign in</a>
    {/if}
  </Row>
</nav>
