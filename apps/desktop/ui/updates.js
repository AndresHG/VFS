// Update notice (VFS-21). One check per launch, a while after the window opens so it never
// competes with the first paint; the backend answers null when the setting is off. The notice
// never takes focus and never blocks: the user can keep searching or dismiss it.

export const UpdatePhase = Object.freeze({
  HIDDEN: 'hidden',
  OFFERED: 'offered',
  DOWNLOADING: 'downloading',
  INSTALLING: 'installing',
  FAILED: 'failed',
  INSTALL_FAILED: 'install-failed',
});

export const CHECK_DELAY_MS = 10_000;

/** What the notice shows for a state, in the catalog `S`: null when hidden. */
export function updateView(st, S) {
  switch (st.phase) {
    case UpdatePhase.OFFERED:
      return { text: S.updateAvailable(st.version), action: S.updateInstall, later: true, progress: null };
    case UpdatePhase.DOWNLOADING: {
      const pct = st.total ? Math.min(100, Math.floor((st.downloaded * 100) / st.total)) : null;
      return { text: S.updateDownloading(pct), action: null, later: false, progress: pct };
    }
    case UpdatePhase.INSTALLING:
      return { text: S.updateInstalling, action: null, later: false, progress: null };
    case UpdatePhase.FAILED:
      return { text: S.updateFailed, action: S.updateRetry, later: true, progress: null };
    case UpdatePhase.INSTALL_FAILED:
      return { text: S.updateInstallFailed, action: S.updateRetry, later: true, progress: null };
    default:
      return null;
  }
}

/**
 * The notice's state machine. `api` needs checkForUpdate() and installUpdate(); `render(view)`
 * gets updateView's result in the catalog `S` after every change; `schedule` is setTimeout
 * (injected for tests).
 */
export function createUpdates({ api, S, render, schedule = setTimeout, delay = CHECK_DELAY_MS }) {
  const st = { phase: UpdatePhase.HIDDEN, version: null, downloaded: 0, total: null };
  const show = () => render(updateView(st, S));

  async function check() {
    try {
      const info = await api.checkForUpdate();
      if (!info) return;
      st.version = info.version;
      st.phase = UpdatePhase.OFFERED;
      show();
    } catch { /* offline or blocked: say nothing, the next launch tries again (VFS-21 R4) */ }
  }

  async function install() {
    Object.assign(st, { phase: UpdatePhase.DOWNLOADING, downloaded: 0, total: null });
    show();
    try {
      // On success the app is replaced and relaunched, so this rarely resolves.
      await api.installUpdate();
      st.phase = UpdatePhase.INSTALLING;
    } catch {
      // Past the download, the failure was the signature check or the installer (VFS-21 R5).
      st.phase = st.phase === UpdatePhase.INSTALLING ? UpdatePhase.INSTALL_FAILED : UpdatePhase.FAILED;
    }
    show();
  }

  function onProgress({ downloaded, total }) {
    if (st.phase !== UpdatePhase.DOWNLOADING) return;
    st.downloaded = downloaded;
    st.total = total;
    if (total && downloaded >= total) st.phase = UpdatePhase.INSTALLING;
    show();
  }

  function dismiss() {
    st.phase = UpdatePhase.HIDDEN;
    show();
  }

  return {
    start: () => schedule(check, delay),
    install,
    onProgress,
    dismiss,
    phase: () => st.phase,
  };
}
