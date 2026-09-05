<script>
  import Logo from "./Logo.svelte";
  import Icon from "./Icon.svelte";
  import { signInHref, signOut } from "../lib/api.js";

  // The bar every page wears: the mark, whatever the page puts in the middle,
  // and who you are.
  //
  // One rule decides the vertical rhythm -- every child of the bar is a flex
  // row centred on the same line, and every control is one square -- so a
  // button added later cannot land half a line above its neighbours. That was
  // the old bug: each control was sized by a rule keyed to whichever ancestor
  // it happened to have, and three of them had different ancestors.
  let { me = {}, children, tools } = $props();
</script>

<nav>
  <ul>
    <li>
      <a class="brand" href="/" aria-label="Komodoc home">
        <Logo />
        <strong class="wordmark"><span class="wordmark-komo">komo</span><span class="wordmark-doc">doc</span></strong>
      </a>
    </li>
    {@render children?.()}
  </ul>
  <!-- The one link that is the same on every page: what Komodoc is and how to
       use it, from the project's own README. -->
  <ul class="navmid">
    <li>
      <a href="/documentation" aria-label="Documentation" title="Documentation">
        <Icon name="book" />
        <span class="navlabel">Documentation</span>
      </a>
    </li>
  </ul>
  <ul>
    {@render tools?.()}
    {#if me.login}
      <li><small class="who">@{me.login}</small></li>
      <li><button type="button" class="signout" onclick={signOut}>Sign out</button></li>
    {:else if me.can_sign_in}
      <li><a role="button" class="signin" href={signInHref()}>Sign in</a></li>
    {/if}
  </ul>
</nav>

<style>
  /* Every item on the bar centres on the same line. The brand contains an SVG
     and the title is bare text, so without this they sit on different
     baselines. A long document name truncates rather than pushing the bar
     around. */
  nav :global(li) {
    display: flex;
    align-items: center;
    min-width: 0;
    padding-top: 0;
    padding-bottom: 0;
  }
  /* The size every control on the bar is, read by IconButton. Set here because
     this is the row they have to line up in. */
  nav {
    --komodoc-control: 2rem;
  }
  .navmid {
    margin: 0 auto;
    flex: 0 0 auto;
  }
  .navmid a {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    color: var(--pico-muted-color);
    white-space: nowrap;
    text-decoration: none;
  }
  .navmid a:hover {
    color: var(--pico-primary);
  }
  .who {
    color: var(--pico-muted-color);
    white-space: nowrap;
  }
  /* The two identity buttons are text rather than icons, and are the height of
     the icons beside them. */
  .signin,
  .signout {
    height: var(--komodoc-control);
    padding: 0 0.75rem;
    margin: 0;
    display: inline-flex;
    align-items: center;
    font-size: 0.85rem;
    line-height: 1;
    white-space: nowrap;
  }
</style>
