import { render } from 'svelte/server';
import App from './App.svelte';

process.stdout.write(render(App).body);
