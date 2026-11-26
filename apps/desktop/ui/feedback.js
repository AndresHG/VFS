// "Enviar comentarios" (VFS-20): the user writes what happened, may read the exact report,
// and saves it (R5) or saves it and opens a prefilled GitHub issue (R6). Nothing is sent by
// the program itself (R8); the report is built in the shell (feedback.rs), always anonymous
// (VFS-48).

/** The message shown once the report is saved: where it is and what to do with it. */
export function savedMessage(S, saved, github) {
  return (github ? S.feedbackSavedGithub : S.feedbackSaved)(saved.folder, saved.file);
}

/** Wires the dialog. `errorText(err)` turns an ApiError into a sentence. */
export function setupFeedback({ api, S, $, errorText }) {
  const dialog = $('feedback');
  const text = $('feedback-text');
  const details = $('feedback-details');
  const report = $('feedback-report');
  const actions = [$('feedback-github'), $('feedback-export')];
  let previewToken = 0;

  function message(textContent, error = false) {
    $('feedback-msg-text').textContent = textContent;
    $('feedback-msg').className = `banner ${error ? 'banner-error' : 'banner-info'}`;
    $('feedback-msg').hidden = false;
    $('feedback-msg').scrollIntoView({ block: 'nearest' });
  }

  // The preview is built only while "Ver el informe" is open, and again when it is opened
  // again, never on typing (the shell reads the whole log for it); a
  // slower, older answer never overwrites a newer one.
  async function refreshPreview() {
    if (!details.open) return;
    const token = ++previewToken;
    report.value = S.feedbackLoading;
    try {
      const value = await api.feedbackReport(text.value);
      if (token === previewToken) report.value = value;
    } catch (err) {
      if (token === previewToken) report.value = errorText(err);
    }
  }

  async function save(github) {
    for (const b of actions) b.disabled = true;
    message(S.feedbackLoading);
    try {
      const saved = await api.feedbackSave(text.value, github);
      message(savedMessage(S, saved, github));
    } catch (err) {
      message(errorText(err), true);
    } finally {
      for (const b of actions) b.disabled = false;
    }
  }

  $('open-feedback').addEventListener('click', () => {
    $('feedback-msg').hidden = true;
    details.open = false;
    report.value = '';
    dialog.showModal();
    text.focus();
  });
  details.addEventListener('toggle', refreshPreview);
  $('feedback-github').addEventListener('click', () => save(true));
  $('feedback-export').addEventListener('click', () => save(false));
}
