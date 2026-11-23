// VFS-39 R2: the index opens off the main thread and its commands reject with `unavailable`
// until it is open, so the UI sends every command that needs the index only after `wait_core`
// has resolved. A click made meanwhile («Añadir carpeta», «Quitar», Ajustes) waits and then
// runs; it never reports a misleading error. When the open fails, those commands reject with
// its error.

/** Commands that need no index: they run at once, also while it opens. */
export const NO_CORE = new Set([
  'wait_core', 'system_locale', 'set_language', 'pick_folder',
  'get_update_settings', 'set_update_check', 'check_for_update', 'install_update',
]);

/** Wraps Tauri's `invoke` so index commands wait for one shared `wait_core`. */
export function gateOnCore(invoke) {
  let open = null;
  const core = () => (open ??= invoke('wait_core'));
  return (cmd, args) => {
    if (cmd === 'wait_core') return core();
    if (NO_CORE.has(cmd)) return invoke(cmd, args);
    return core().then(() => invoke(cmd, args));
  };
}
