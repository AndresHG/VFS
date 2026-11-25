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

/** What this launch's check found (VFS-45 R5): not run yet, nothing newer, an update, or a
 *  failure (the shell rejects a failed check). */
export const CheckResult = Object.freeze({
  NOT_RUN: 'not-run',
  NONE: 'none',
  FOUND: 'found',
  FAILED: 'failed',
});

/** The button of the «Acerca de» update line (VFS-45 R5). */
export const AboutAction = Object.freeze({ INSTALL: 'install', CHECK: 'check' });

/**
 * The update line of «Acerca de» (VFS-45 R5) in the catalog `S`: `text`, the `action` button
 * («Actualizar» to install, «Buscar actualizaciones» to check now) or null. `checkOn` is the
 * VFS-21 setting: while it is off nothing is offered, so nothing connects.
 */
export function aboutUpdateView({ checkOn, result, version }, S) {
  if (!checkOn) return { text: S.aboutUpdatesOff, action: null };
  switch (result) {
    case CheckResult.FOUND: return { text: S.aboutUpdateFound(version), action: AboutAction.INSTALL };
    case CheckResult.NONE: return { text: S.aboutUpToDate, action: null };
    case CheckResult.FAILED: return { text: S.aboutUpdateFailed, action: AboutAction.CHECK };
    default: return { text: '', action: AboutAction.CHECK };
  }
}

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
export function createUpdates({ api, S, render, schedule = setTimeout, delay = CHECK_DELAY_MS, checkOn = () => true }) {
  const st = { phase: UpdatePhase.HIDDEN, version: null, downloaded: 0, total: null, result: CheckResult.NOT_RUN };
  const show = () => render(updateView(st, S));
  let running = null;

  /** One check; the shell runs it at most once per launch and answers the same after. While
   *  the setting is off (`checkOn`) the shell answers null without checking, which is not
   *  "nothing newer": the result stays not run, so «Acerca de» offers the check once the
   *  setting is turned on. */
  function check() {
    running ??= (async () => {
      try {
        const on = checkOn();
        const info = await api.checkForUpdate();
        if (!info) {
          if (on) st.result = CheckResult.NONE;
          return;
        }
        st.result = CheckResult.FOUND;
        st.version = info.version;
        if (st.phase === UpdatePhase.HIDDEN) {
          st.phase = UpdatePhase.OFFERED;
          show();
        }
      } catch {
        // Offline or blocked: say nothing, the next launch tries again (VFS-21 R4).
        st.result = CheckResult.FAILED;
      } finally {
        running = null;
      }
    })();
    return running;
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
    /** «Buscar actualizaciones» in «Acerca de» (VFS-45 R5): check now and settle. */
    checkNow: check,
    /** This launch's check result and the version it found, for «Acerca de». */
    result: () => ({ result: st.result, version: st.version }),
    phase: () => st.phase,
  };
}
