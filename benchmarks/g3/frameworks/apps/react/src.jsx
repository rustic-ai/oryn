import React, {useEffect, useState} from 'react';
import {createPortal} from 'react-dom';
import {hydrateRoot} from 'react-dom/client';

class G3Badge extends HTMLElement {
  connectedCallback() {
    this.textContent = this.getAttribute('label') || 'Custom element ready';
  }
}
if (!customElements.get('g3-badge')) customElements.define('g3-badge', G3Badge);

const products = [
  {id: 1, name: 'Oryn Keyboard', category: 'tools'},
  {id: 2, name: 'Semantic Mouse', category: 'tools'},
  {id: 3, name: 'Browser Guide', category: 'books'},
  {id: 4, name: 'Runtime Handbook', category: 'books'},
];

function App() {
  const [enhanced, setEnhanced] = useState(false);
  const [count, setCount] = useState(0);
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('all');
  const [date, setDate] = useState('');
  const [page, setPage] = useState(1);
  const [cart, setCart] = useState(0);
  const [upload, setUpload] = useState('none');
  const [chunk, setChunk] = useState('idle');
  const [modal, setModal] = useState(false);

  useEffect(() => setEnhanced(true), []);

  const visibleProducts = products.filter(product =>
    (category === 'all' || product.category === category) &&
    product.name.toLowerCase().includes(query.toLowerCase())
  );
  const add = () => {
    setCount(value => value + 1);
    localStorage.setItem('react-count', String(count + 1));
  };
  const loadChunk = async () => {
    const module = await import('./surface-chunk.js');
    setChunk(module.surfaceLabel);
  };
  const attachFixture = () => {
    const file = new File(['fixture'], 'fixture.txt', {type: 'text/plain'});
    setUpload(`${file.name}:${file.size}`);
  };

  return <main data-framework="react">
    <h1>React ready</h1>
    <label>Search<input aria-label="Search" value={query} onInput={event => setQuery(event.target.value)}/></label>
    <button onClick={add}>Add item</button>
    <output id="items">Items: {count}</output>
    <a href="?route=details">Route</a>
    {enhanced && <section id="g3-modern-surface">
      <label>Category<select aria-label="Category" value={category} onChange={event => setCategory(event.target.value)}><option value="all">All</option><option value="tools">Tools</option><option value="books">Books</option></select></label>
      <label>Delivery date<input type="date" aria-label="Delivery date" value={date} onInput={event => setDate(event.target.value)}/></label>
      <button onClick={attachFixture}>Attach fixture file</button>
      <button onClick={loadChunk}>Load chunk</button>
      <button onClick={() => setModal(true)}>Open modal</button>
      <button onClick={() => setPage(value => value + 1)}>Next page</button>
      <button onClick={() => setCart(value => value + 1)}>Buy</button>
      <a download="oryn-report.json" href="data:application/json,%7B%22oryn%22%3Atrue%7D">Download report</a>
      <g3-badge label="Custom element ready"></g3-badge>
      <table><thead><tr><th>Product</th><th>Category</th></tr></thead><tbody>{visibleProducts.map(product => <tr key={product.id}><td>{product.name}</td><td>{product.category}</td></tr>)}</tbody></table>
      <output id="g3-surface">Surface: {query}|{category}|{date}|{page}|{cart}|{upload}|{chunk}|{modal ? 'open' : 'closed'}</output>
      {modal && createPortal(<div role="dialog" aria-modal="true"><h2>Portal modal</h2><button onClick={() => setModal(false)}>Close modal</button></div>, document.getElementById('modal-root'))}
    </section>}
  </main>;
}

hydrateRoot(document.getElementById('app'), <App/>);
