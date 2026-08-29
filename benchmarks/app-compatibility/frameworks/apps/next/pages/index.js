import Head from 'next/head';
import Link from 'next/link';
import { useState } from 'react';

export default function Home() {
  const [count, setCount] = useState(0);
  const [query, setQuery] = useState('');
  return <>
    <Head><title>Next application compatibility</title></Head>
    <main data-framework="next"><h1>Next ready</h1><label>Search<input aria-label="Search" value={query} onInput={event => setQuery(event.target.value)} /></label><button onClick={() => { setCount(value => value + 1); localStorage.setItem('next-count', String(count + 1)); }}>Add item</button><output>Items: {count}</output><Link href="/details">Route</Link></main>
  </>;
}
