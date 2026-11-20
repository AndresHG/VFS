// Fixed-row-height virtual list: only the rows in view (plus a margin) exist in the DOM, so
// thousands of results stay smooth on a slow PC. The row height is measured from a real row,
// and re-measured when the text size or the window changes, so it follows the user's text size.

const OVERSCAN = 8;

export function createVirtualList({ scroller, sizer, buildRow, fillRow }) {
  let count = 0;
  let rowH = 0;
  let frame = 0;
  const live = new Map(); // index -> element

  function measure() {
    const probe = buildRow();
    probe.style.visibility = 'hidden';
    probe.removeAttribute('id');
    probe.removeAttribute('role');
    sizer.append(probe);
    const h = probe.getBoundingClientRect().height;
    probe.remove();
    if (h && Math.abs(h - rowH) > 0.5) {
      rowH = h;
      sizer.style.height = `${count * rowH}px`;
      for (const [i, el] of live) el.style.transform = `translateY(${i * rowH}px)`;
      return true;
    }
    return false;
  }

  function render() {
    frame = 0;
    if (!rowH) measure();
    if (!rowH) return;
    const top = scroller.scrollTop;
    const first = Math.max(0, Math.floor(top / rowH) - OVERSCAN);
    const last = Math.min(count - 1, Math.ceil((top + scroller.clientHeight) / rowH) + OVERSCAN);
    for (const [i, el] of live) {
      if (i < first || i > last) { el.remove(); live.delete(i); }
    }
    for (let i = first; i <= last; i++) {
      if (live.has(i)) continue;
      const el = buildRow();
      el.style.transform = `translateY(${i * rowH}px)`;
      fillRow(el, i);
      live.set(i, el);
      sizer.append(el);
    }
  }

  function schedule() {
    if (!frame) frame = requestAnimationFrame(render);
  }

  scroller.addEventListener('scroll', schedule, { passive: true });
  // Width changes can change the row height (text scaling, layout breakpoints).
  new ResizeObserver(() => { measure(); schedule(); }).observe(scroller);
  matchMedia('(resolution: 1dppx)').addEventListener?.('change', () => { measure(); schedule(); });

  return {
    /** Replaces the whole list. */
    reset(n) {
      count = n;
      for (const el of live.values()) el.remove();
      live.clear();
      sizer.style.height = `${count * rowH}px`;
      scroller.scrollTop = 0;
      render();
    },
    /** Re-fills rows currently in the DOM (data arrived, selection moved). */
    refresh(indices) {
      if (indices) { for (const i of indices) { const el = live.get(i); if (el) fillRow(el, i); } }
      else for (const [i, el] of live) fillRow(el, i);
    },
    /** Scrolls the least needed to bring row `i` fully into view. */
    reveal(i) {
      if (!rowH) return;
      const top = i * rowH;
      if (top < scroller.scrollTop) scroller.scrollTop = top;
      else if (top + rowH > scroller.scrollTop + scroller.clientHeight) {
        scroller.scrollTop = top + rowH - scroller.clientHeight;
      }
      render();
    },
    pageSize() { return rowH ? Math.max(1, Math.floor(scroller.clientHeight / rowH) - 1) : 10; },
    atTop() { return scroller.scrollTop < 4; },
    remeasure() { if (measure()) schedule(); },
    element(i) { return live.get(i); },
  };
}
