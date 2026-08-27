<script>let count=0;let query='';function add(){count+=1;localStorage.setItem('svelte-count',String(count));}</script>
<main data-framework="svelte"><h1>Svelte ready</h1><label>Search<input aria-label="Search" bind:value={query}></label><button on:click={add}>Add item</button><output>Items: {count}</output><a href="?route=details">Route</a></main>
