//! The shell's own strings in Spanish and English (VFS-22 R5): the macOS app menu, the window
//! title, the folder picker, the startup error and the Windows splash. The UI's catalogs are
//! `ui/strings.es.js` and `ui/strings.en.js`; these few strings are the ones Rust draws itself.
//! Before the UI runs (splash, startup error, first menu) the language follows the OS locale
//! with the UI's rule; afterwards the UI sends the active language through `set_language`.

use std::sync::Mutex;

use tauri::menu::{Menu, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Runtime};

use crate::MAIN_WINDOW;

/// A UI language; its code is what the UI sends (`"es"`, `"en"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    Es,
    En,
}

impl Lang {
    /// VFS-22 R2: Spanish for `es` and `es-*` (also `es_ES.UTF-8`), English for anything else.
    /// Same rule as `languageFor` in `ui/strings.js`.
    pub fn for_locale(locale: &str) -> Self {
        let primary = locale.split(['-', '_', '.']).next().unwrap_or("");
        if primary.eq_ignore_ascii_case("es") {
            Lang::Es
        } else {
            Lang::En
        }
    }

    /// The language of the OS locale.
    pub fn system() -> Self {
        Self::for_locale(&sys_locale::get_locale().unwrap_or_default())
    }

    /// The language with UI code `code`.
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "es" => Some(Lang::Es),
            "en" => Some(Lang::En),
            _ => None,
        }
    }

    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::Es => &ES,
            Lang::En => &EN,
        }
    }
}

/// The shell's strings in one language. `{app}` stands for the product name.
#[derive(Debug)]
pub struct Strings {
    pub window_title: &'static str,
    pub choose_folder: &'static str,
    pub splash: &'static str,
    /// `{detail}` stands for the error.
    pub startup_failed: &'static str,
    pub menu: MenuStrings,
}

/// The macOS app menu, in the words macOS itself uses in that language.
#[derive(Debug)]
pub struct MenuStrings {
    pub about: &'static str,
    pub services: &'static str,
    pub hide: &'static str,
    pub hide_others: &'static str,
    pub show_all: &'static str,
    pub quit: &'static str,
    pub edit: &'static str,
    pub undo: &'static str,
    pub redo: &'static str,
    pub cut: &'static str,
    pub copy: &'static str,
    pub paste: &'static str,
    pub select_all: &'static str,
    pub view: &'static str,
    pub fullscreen: &'static str,
    pub window: &'static str,
    pub minimize: &'static str,
    pub zoom: &'static str,
    pub close_window: &'static str,
}

static ES: Strings = Strings {
    window_title: "VFS — Buscador de archivos",
    choose_folder: "Elegir carpeta",
    splash: "Preparando VFS…",
    startup_failed: "No se ha podido iniciar VFS.\n\nDetalle: {detail}\n\n\
        Cierre el programa y vuelva a abrirlo. Si el problema continúa, \
        comuníquelo junto con este mensaje.",
    menu: MenuStrings {
        about: "Acerca de {app}",
        services: "Servicios",
        hide: "Ocultar {app}",
        hide_others: "Ocultar otros",
        show_all: "Mostrar todo",
        quit: "Salir de {app}",
        edit: "Edición",
        undo: "Deshacer",
        redo: "Rehacer",
        cut: "Cortar",
        copy: "Copiar",
        paste: "Pegar",
        select_all: "Seleccionar todo",
        view: "Visualización",
        fullscreen: "Pantalla completa",
        window: "Ventana",
        minimize: "Minimizar",
        zoom: "Zoom",
        close_window: "Cerrar ventana",
    },
};

static EN: Strings = Strings {
    window_title: "VFS — File Finder",
    choose_folder: "Choose folder",
    splash: "Getting VFS ready…",
    startup_failed: "VFS could not start.\n\nDetails: {detail}\n\n\
        Close the app and open it again. If the problem continues, \
        report it together with this message.",
    menu: MenuStrings {
        about: "About {app}",
        services: "Services",
        hide: "Hide {app}",
        hide_others: "Hide Others",
        show_all: "Show All",
        quit: "Quit {app}",
        edit: "Edit",
        undo: "Undo",
        redo: "Redo",
        cut: "Cut",
        copy: "Copy",
        paste: "Paste",
        select_all: "Select All",
        view: "View",
        fullscreen: "Enter Full Screen",
        window: "Window",
        minimize: "Minimize",
        zoom: "Zoom",
        close_window: "Close Window",
    },
};

/// The language the shell draws in (managed state).
pub struct ShellLang(pub Mutex<Lang>);

/// The shell's current language: the UI's, once it has sent it; the OS's before that.
pub fn current<R: Runtime>(app: &AppHandle<R>) -> Lang {
    app.try_state::<ShellLang>()
        .and_then(|s| s.0.lock().ok().map(|l| *l))
        .unwrap_or_else(Lang::system)
}

/// The macOS app menu in `lang`: the app, Edit (the web view's copy and paste need it), View
/// and Window menus of Tauri's default menu, with every label in `lang`.
pub fn menu<R: Runtime>(app: &AppHandle<R>, lang: Lang) -> tauri::Result<Menu<R>> {
    let m = &lang.strings().menu;
    let name = app.package_info().name.clone();
    let with_app = |s: &str| s.replace("{app}", &name);
    let item = |f: fn(&AppHandle<R>, Option<&str>) -> tauri::Result<PredefinedMenuItem<R>>,
                text: &str| f(app, Some(text));
    let sep = || PredefinedMenuItem::separator(app);
    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                &name,
                true,
                &[
                    &PredefinedMenuItem::about(app, Some(&with_app(m.about)), None)?,
                    &sep()?,
                    &item(PredefinedMenuItem::services, m.services)?,
                    &sep()?,
                    &item(PredefinedMenuItem::hide, &with_app(m.hide))?,
                    &item(PredefinedMenuItem::hide_others, m.hide_others)?,
                    &item(PredefinedMenuItem::show_all, m.show_all)?,
                    &sep()?,
                    &item(PredefinedMenuItem::quit, &with_app(m.quit))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                m.edit,
                true,
                &[
                    &item(PredefinedMenuItem::undo, m.undo)?,
                    &item(PredefinedMenuItem::redo, m.redo)?,
                    &sep()?,
                    &item(PredefinedMenuItem::cut, m.cut)?,
                    &item(PredefinedMenuItem::copy, m.copy)?,
                    &item(PredefinedMenuItem::paste, m.paste)?,
                    &item(PredefinedMenuItem::select_all, m.select_all)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                m.view,
                true,
                &[&item(PredefinedMenuItem::fullscreen, m.fullscreen)?],
            )?,
            &Submenu::with_items(
                app,
                m.window,
                true,
                &[
                    &item(PredefinedMenuItem::minimize, m.minimize)?,
                    &item(PredefinedMenuItem::maximize, m.zoom)?,
                    &sep()?,
                    &item(PredefinedMenuItem::close_window, m.close_window)?,
                ],
            )?,
        ],
    )
}

/// Makes `lang` the shell's language: window title, and on macOS the app menu.
pub fn apply<R: Runtime>(app: &AppHandle<R>, lang: Lang) -> tauri::Result<()> {
    if let Some(state) = app.try_state::<ShellLang>() {
        if let Ok(mut l) = state.0.lock() {
            *l = lang;
        }
    }
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        w.set_title(lang.strings().window_title)?;
    }
    if cfg!(target_os = "macos") {
        app.set_menu(menu(app, lang)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-22 AC-3: the shell picks the language from the OS locale with the UI's rule.
    #[test]
    fn os_locale_picks_the_language() {
        for l in ["es-ES", "es-MX", "es", "es_ES.UTF-8", "ES-es"] {
            assert_eq!(Lang::for_locale(l), Lang::Es, "{l}");
        }
        for l in ["en-US", "fr-FR", "ca-ES", "eu-ES", "est", ""] {
            assert_eq!(Lang::for_locale(l), Lang::En, "{l}");
        }
        assert_eq!(Lang::from_code("en"), Some(Lang::En));
        assert_eq!(Lang::from_code("system"), None);
    }

    /// VFS-22 AC-6: the menu words are Spanish in Spanish and English in English (both
    /// languages fill the same struct, so the compiler checks that every string exists).
    #[test]
    fn shell_strings_exist_in_both_languages() {
        assert_eq!(Lang::Es.strings().menu.edit, "Edición");
        assert_eq!(Lang::Es.strings().menu.window, "Ventana");
        assert_eq!(Lang::En.strings().menu.edit, "Edit");
        assert_eq!(Lang::En.strings().menu.window, "Window");
        for s in [Lang::Es.strings(), Lang::En.strings()] {
            assert!(s.startup_failed.contains("{detail}"));
            assert!(s.menu.quit.contains("{app}"));
        }
    }

    /// VFS-22 AC-7 (issue 112): the installer offers Spanish and English and names the
    /// publisher "VFS"; the configured window title is the Spanish one the shell also sets.
    #[test]
    fn installer_languages_and_publisher() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["bundle"]["publisher"], "VFS");
        let langs = &conf["bundle"]["windows"]["nsis"]["languages"];
        assert_eq!(langs, &serde_json::json!(["Spanish", "English"]));
        assert_eq!(
            conf["app"]["windows"][0]["title"],
            Lang::Es.strings().window_title
        );
    }
}
