// «Relacionados» (VFS-53 R4–R7): the documents closest to one document by meaning, each with
// why («mismo tema», «misma carpeta») and its closest passage, under a hit of «Documentos» and
// in the Chat source panel. `reasonsFor` has no DOM, so it is tested outside the browser
// (`apps/desktop/ui-dev/related.test.mjs`).

/** Why a related document shows (R6): always the same topic; the same folder when it is. */
export function reasonsFor(S, hit, dir) {
  const reasons = [S.relatedSameTopic];
  if (dir != null && hit.dir === dir) reasons.push(S.relatedSameFolder);
  return reasons;
}

/** Search by meaning serves (VFS-29 R11): switched on and a complete vector set exists. */
export function relatedAvailable(meaning) {
  return !!(meaning?.settings?.enabled && meaning?.searchable);
}

/**
 * A box that loads and shows the related documents of `fileId` (in folder `dir`): a line while
 * it loads, the list, or one line when there is none (R7) or the request failed.
 */
export function relatedBox({ api, S, el, revealFile, fileId, dir }) {
  const box = el('div', 'related-box');
  box.setAttribute('aria-live', 'polite');
  box.append(el('p', 'related-note', S.relatedLoading));
  api.relatedDocuments(fileId).then((hits) => {
    if (!hits.length) {
      box.replaceChildren(el('p', 'related-note', S.relatedNone));
      return;
    }
    const list = el('ul', 'related-list');
    for (const hit of hits) {
      const li = el('li', 'related-item');
      const main = el('div', 'main');
      const reasons = el('span', 'related-reasons', reasonsFor(S, hit, dir).join(' · '));
      main.append(el('span', 'name', hit.name), el('span', 'path', hit.dir), el('p', 'fragment', hit.snippet), reasons);
      const show = el('button', 'btn btn-quiet', S.revealButton);
      show.type = 'button';
      show.setAttribute('aria-label', `${S.revealButton}: ${hit.name}`);
      show.addEventListener('click', () => revealFile(hit.id, false));
      li.append(main, show);
      list.append(li);
    }
    box.replaceChildren(el('p', 'related-head', S.relatedTitle), list);
  }).catch(() => box.replaceChildren(el('p', 'related-note', S.relatedFailed)));
  return box;
}

/** A button that opens the related box (`open` adds it and returns it), and removes it on a
 *  second press. */
export function relatedToggle({ label, el, open }) {
  const btn = el('button', 'link-btn related-toggle', label);
  btn.type = 'button';
  btn.setAttribute('aria-expanded', 'false');
  let box = null;
  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    if (box) {
      box.remove();
      box = null;
      btn.setAttribute('aria-expanded', 'false');
      return;
    }
    box = open();
    btn.setAttribute('aria-expanded', 'true');
  });
  return btn;
}
