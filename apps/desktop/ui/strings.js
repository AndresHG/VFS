// Every user-visible string (VFS-1 R9) in Spanish and English (VFS-22). The catalogs are
// `strings.es.js` and `strings.en.js`, with the same keys (ui-dev/strings.test.mjs).
// `localeFor` gives one language's catalog `S` and formatters `N`; the screen owns that value
// and passes it on, so no module holds the active language.

import { es } from './strings.es.js';
import { en } from './strings.en.js';
import { formatsFor } from './format.js';

/** The UI languages; also the values a saved choice may hold. */
export const Lang = Object.freeze({ ES: 'es', EN: 'en' });

export const CATALOGS = Object.freeze({ [Lang.ES]: es, [Lang.EN]: en });

/** The `Intl` locale of a language when the system locale is in another one. */
const DEFAULT_INTL = Object.freeze({ [Lang.ES]: 'es-ES', [Lang.EN]: 'en-US' });

/** BCP 47 form of a system locale: `es_ES.UTF-8` -> `es-ES`. */
function bcp47(locale) {
  return String(locale || '').split('.')[0].replace(/_/g, '-');
}

/** VFS-22 R2: a saved choice wins; otherwise Spanish for `es` and `es-*`, English otherwise. */
export function languageFor(systemLocale, saved) {
  if (Object.hasOwn(CATALOGS, saved)) return saved;
  return /^es(-|$)/i.test(bcp47(systemLocale)) ? Lang.ES : Lang.EN;
}

/** The `Intl` locale for dates and numbers: the system's own when it is in `lang` (so en-GB
 *  keeps day-month order), otherwise the language's default. */
export function intlLocale(lang, systemLocale) {
  const tag = bcp47(systemLocale);
  if (tag.toLowerCase().split('-')[0] === lang) {
    try {
      return Intl.getCanonicalLocales(tag)[0];
    } catch {
      // not a valid tag: fall through to the default
    }
  }
  return DEFAULT_INTL[lang];
}

/** Everything language-dependent the screen needs: `lang`, the catalog `S` and the number
 *  and date formatters `N`. An unknown `lang` is Spanish. */
export function localeFor(lang, systemLocale) {
  const l = Object.hasOwn(CATALOGS, lang) ? lang : Lang.ES;
  return Object.freeze({ lang: l, S: CATALOGS[l], N: formatsFor(l, intlLocale(l, systemLocale)) });
}
