// Side panels close on a click outside them, as on Esc. WebKit (the macOS shell) has no
// `closedby="any"`, so the panels marked `data-light-dismiss` in index.html get it from here, in
// every engine alike; their `closedby="closerequest"` keeps Esc only.
// Unsaved input is not lost: the field being edited is blurred first, so a number that applies
// on change (the AI monthly cap, the content size limit) is applied; text typed and not yet
// added or saved (a filter pattern, a key, a comment) stays in its field for the next open.

/** Makes a click on the backdrop of `dialog` close it. A press that starts inside the panel
 *  (selecting text and releasing outside) does not count. */
export function lightDismiss(dialog) {
  const outside = (e) => {
    if (e.target !== dialog) return false;
    const r = dialog.getBoundingClientRect();
    return e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom;
  };
  const commit = () => { if (dialog.contains(document.activeElement)) document.activeElement.blur(); };
  let pressedOutside = false;
  dialog.addEventListener('pointerdown', (e) => { pressedOutside = outside(e); });
  dialog.addEventListener('click', (e) => {
    if (!pressedOutside || !outside(e)) return;
    pressedOutside = false;
    commit();
    dialog.close();
  });
  dialog.addEventListener('cancel', commit);
}
