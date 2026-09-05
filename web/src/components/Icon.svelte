<script>
  // The icons the pages use, from Lucide, as paths rather than as a font or a
  // sprite request: two small paths cost less than either. See
  // styles/lucide-LICENSE.txt.
  const PATHS = {
    book: ['M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H19a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6.5a1 1 0 0 1 0-5H20'],
    "panel-left": ['M3 3h18v18H3z', 'M9 3v18'],
    "file-text": ['M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z', 'M14 2v4a2 2 0 0 0 2 2h4', 'M10 9H8', 'M16 13H8', 'M16 17H8'],
    "panel-right": ['M3 3h18v18H3z', 'M15 3v18'],
    lock: ['M3 11h18v11H3z', 'M7 11V7a5 5 0 0 1 10 0v4'],
    save: ['M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z', 'M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7', 'M7 3v4a1 1 0 0 0 1 1h7'],
    link: ['M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71', 'M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71'],
    check: ['M20 6 9 17l-5-5'],
    comment: ['M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z', 'M7 11h10', 'M7 15h6', 'M7 7h8'],
    highlight: ['m9 11-6 6v3h9l3-3', 'm22 12-4.6 4.6a2 2 0 0 1-2.8 0l-5.2-5.2a2 2 0 0 1 0-2.8L14 4'],
    box: ['M5 3a2 2 0 0 0-2 2', 'M19 3a2 2 0 0 1 2 2', 'M5 21a2 2 0 0 1-2-2', 'M9 3h1', 'M9 21h2', 'M14 3h1', 'M3 9v1', 'M21 9v2', 'M3 14v1', 'm21 15 -3 3 -2 -2 -3 3v-8z'],
  };

  let { name } = $props();
</script>

<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
     stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#each PATHS[name] ?? [] as d}
    <path {d} />
  {/each}
</svg>
