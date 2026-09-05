<script>
  import Logo from "./Logo.svelte";
  import Icon from "./Icon.svelte";
  import { signInHref, signOut } from "../lib/api.js";

  // The bar every page wears: the mark, whatever the page puts in the middle,
  // and who you are. The identity block is the same pair in the same order on
  // the landing page and in the reader.
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
      <li><small id="who">@{me.login}</small></li>
      <li><button type="button" onclick={signOut}>Sign out</button></li>
    {:else if me.can_sign_in}
      <li><a role="button" href={signInHref()}>Sign in</a></li>
    {/if}
  </ul>
</nav>
