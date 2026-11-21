// The Spanish catalog (VFS-1 R9, VFS-22 R1). Impersonal/usted, plain words: never "indexar",
// "índice" or "reindexar" (VFS-1 decision 16). Functions take already-formatted values. Every
// key exists in `strings.en.js` too (ui-dev/strings.test.mjs).

import { DateChoice, LanguageChoice, SizeChoice, SourceChoice } from './choices.js';
import { BookmarkRoot, ErrorCode, FileKind, ReadState, RootStatus, SortOrder } from './contract.js';

export const es = {
  appName: 'Buscador de archivos',
  windowTitle: 'VFS — Buscador de archivos',
  searchLabel: 'Buscar por nombre de archivo o de carpeta',
  searchPlaceholder: 'Por ejemplo: escritura notaría',
  searchHint: 'Use las flechas arriba y abajo para recorrer los resultados, Intro para mostrar el elegido en su carpeta y Escape para borrar la búsqueda.',
  clearSearch: 'Borrar la búsqueda',
  devBadge: 'Datos de prueba',

  filtersTitle: 'Filtros',
  fType: 'Tipo',
  fDate: 'Fecha',
  fSize: 'Tamaño',
  fFolder: 'Carpeta',
  fFrom: 'Desde',
  fTo: 'Hasta',
  // The type button names what is chosen: "Todos los tipos", "Fotos, Vídeos", "Fotos y 2 más".
  typeButton: (...names) => (!names.length ? 'Todos los tipos'
    : names.length <= 2 ? names.join(', ') : `${names[0]} y ${names.length - 1} más`),
  clearFilters: 'Quitar filtros',
  clearFiltersN: (n) => `Quitar filtros (${n})`,
  rangeInverted: 'La fecha «Desde» es posterior a «Hasta».',

  kindsPlural: {
    [FileKind.DOCUMENT]: 'Documentos', [FileKind.PHOTO]: 'Fotos', [FileKind.VIDEO]: 'Vídeos',
    [FileKind.AUDIO]: 'Audio', [FileKind.FOLDER]: 'Carpetas', [FileKind.OTHER]: 'Otros',
  },
  kindSingular: {
    [FileKind.DOCUMENT]: 'Documento', [FileKind.PHOTO]: 'Foto', [FileKind.VIDEO]: 'Vídeo',
    [FileKind.AUDIO]: 'Audio', [FileKind.FOLDER]: 'Carpeta', [FileKind.OTHER]: 'Otro',
  },

  // Option order is display order.
  dateOptions: {
    [DateChoice.ANY]: 'Cualquier fecha', [DateChoice.WEEK]: 'Últimos 7 días',
    [DateChoice.MONTH]: 'Últimos 30 días', [DateChoice.YEAR]: 'Último año',
    [DateChoice.CUSTOM]: 'Elegir fechas…',
  },
  sizeOptions: {
    [SizeChoice.ANY]: 'Cualquier tamaño', [SizeChoice.SMALL]: 'Menos de 1 MB',
    [SizeChoice.MEDIUM]: 'De 1 MB a 100 MB', [SizeChoice.LARGE]: 'De 100 MB a 1 GB',
    [SizeChoice.HUGE]: 'Más de 1 GB',
  },
  fSource: 'Buscar en',
  sourceOptions: {
    [SourceChoice.BOTH]: 'Archivos y marcadores', [SourceChoice.FILES]: 'Archivos',
    [SourceChoice.BOOKMARKS]: 'Marcadores',
  },
  fSort: 'Orden',
  sortOptions: {
    [SortOrder.RELEVANCE]: 'Lo más parecido primero', [SortOrder.NEWEST]: 'Lo más reciente primero',
    [SortOrder.SOURCE]: 'Archivos primero, luego marcadores',
  },
  folderGroupBookmarks: 'Marcadores de Firefox',
  folderAll: 'Todas las carpetas',
  folderPick: 'Elegir otra carpeta…',
  folderInside: (name) => `Dentro de: ${name}`,

  resultsTitle: 'Resultados',
  results: (n, one) => (one ? '1 resultado' : `${n} resultados`),
  resultsCapped: (n) => `Más de ${n} resultados. Escriba algo más para afinar.`,
  resultsPartial: (n) => `${n} resultados. La búsqueda no terminó del todo; puede que falte alguno.`,
  searching: 'Buscando…',
  fuzzyNote: 'No hay coincidencias exactas; se muestran resultados parecidos.',
  loadingRow: 'Cargando…',
  revealButton: 'Mostrar en carpeta',
  revealKey: 'Intro',
  unavailableBadge: 'No disponible',
  openButton: 'Abrir en Firefox',
  folderContentsButton: 'Ver su contenido',
  bookmarkKind: 'Marcador',
  bookmarkFolderKind: 'Carpeta de marcadores',
  bookmarkRoots: {
    [BookmarkRoot.MENU]: 'Menú de marcadores', [BookmarkRoot.TOOLBAR]: 'Barra de marcadores',
    [BookmarkRoot.UNFILED]: 'Otros marcadores', [BookmarkRoot.MOBILE]: 'Marcadores del móvil',
  },
  bookmarkRowLabel: (title, kind, path, site, date) =>
    [title, kind, `en ${path}`, site, date && `guardado el ${date}`].filter(Boolean).join(', '),
  detailsBookmark: 'Dirección:',
  detailsBookmarkFolder: 'Guardado en:',
  rowLabel: (name, kind, dir, date, size, unavailable, places = 1) =>
    [name, kind, `en ${dir}`, date && `modificado el ${date}`, size, unavailable && 'no disponible',
      places > 1 && `está en ${places} sitios, Mayúsculas más Intro para verlos`]
      .filter(Boolean).join(', '),
  // VFS-19: copies of a file (same name and size) shown as one result.
  copiesBadge: (n) => `en ${n} sitios`,
  copiesBadgeLabel: (n) => `Ver los ${n} sitios donde está`,
  copiesTitle: (name, n) => `«${name}» está en ${n} sitios`,
  copiesIntro: 'Son archivos con el mismo nombre y el mismo tamaño: casi seguro, copias del mismo archivo. Elija cuál quiere ver.',
  copiesModified: (date) => `Modificado el ${date}`,
  detailsLabel: 'Ubicación:',

  emptyNoRootsTitle: 'Añada una carpeta para empezar',
  emptyNoRootsBody: 'Elija la carpeta donde guarda sus archivos. El programa la revisará y después podrá buscar en ella. Sus archivos no se mueven ni se modifican.',
  emptyFirstScanTitle: 'Preparando la búsqueda…',
  emptyFirstScanBody: (n) => `Ya se han revisado ${n} archivos. Puede buscar mientras tanto; si cierra el programa, continuará al abrirlo.`,
  emptyIdleTitle: (n) => `Escriba para buscar entre ${n} archivos`,
  emptyIdleBody: 'Puede escribir parte del nombre o de la carpeta. No importan las mayúsculas, los acentos ni una letra equivocada.',
  emptyNoHitsTitle: (q) => (q ? `No se encontró nada para «${q}»` : 'No hay archivos con estos filtros'),
  emptyNoHitsBody: 'Compruebe lo escrito o pruebe con menos palabras.',
  emptyNoHitsFilters: 'Hay filtros puestos que pueden estar ocultando resultados.',
  emptyDeadlineTitle: 'La búsqueda no terminó; inténtelo de nuevo.',
  emptyDeadlineBody: 'El equipo está muy ocupado ahora mismo. Si vuelve a pasar, escriba alguna palabra más.',
  emptyErrorTitle: 'No se pudo hacer la búsqueda',
  emptyErrorBody: 'Inténtelo de nuevo dentro de un momento.',
  retry: 'Intentar de nuevo',
  addFolderEllipsis: 'Añadir carpeta…',

  firstScanNote: 'Se está preparando la búsqueda. Puede buscar mientras tanto; si cierra el programa, continuará al abrirlo.',
  unavailableOne: (p) => `No se encuentra la carpeta «${p}». ¿Está conectado el disco? Sus archivos siguen en la lista, marcados como «No disponible».`,
  quoted: (s) => `«${s}»`,
  unavailableMany: (list) => `No se encuentran estas carpetas: ${list}. ¿Están conectados los discos? Sus archivos siguen en la lista, marcados como «No disponible».`,
  understood: 'Entendido',
  dismiss: 'Cerrar aviso',

  statusScanning: (files, dirs) => `Revisando carpetas… ${files} archivos encontrados · quedan ${dirs} carpetas`,
  statusChecking: (files) => `Comprobando cambios… · ${files} archivos`,
  statusPaused: (files) => `Revisión en pausa · ${files} archivos encontrados`,
  statusChecked: (when, files) => `Última revisión: ${when} · ${files} archivos`,
  statusFiles: (files) => `${files} archivos`,
  today: (time) => `hoy ${time}`,
  yesterday: (time) => `ayer ${time}`,
  dayAndTime: (day, time) => `${day}, ${time}`,
  statusReadyUnavailable: (files, n) => `${files} archivos · ${n === 1 ? '1 carpeta no disponible' : `${n} carpetas no disponibles`}`,
  statusNoRoots: 'Todavía no hay carpetas',
  statusConnecting: 'Conectando…',
  pause: 'Pausar',
  resume: 'Continuar',
  foldersButton: (n) => (n ? `Carpetas (${n})` : 'Carpetas'),

  foldersTitle: 'Carpetas donde se busca',
  foldersIntro: 'El programa solo busca dentro de estas carpetas y de todo lo que contienen. Nunca mueve, cambia ni borra sus archivos.',
  foldersEmpty: 'Todavía no ha añadido ninguna carpeta.',
  addFolder: 'Añadir carpeta…',
  rescanAll: 'Buscar cambios ahora',
  rescan: 'Volver a revisar',
  remove: 'Quitar',
  close: 'Cerrar',
  rootStatus: {
    [RootStatus.PENDING]: 'En espera', [RootStatus.SCANNING]: 'Revisando…', [RootStatus.READY]: 'Lista',
    [RootStatus.UNAVAILABLE]: 'No disponible', [RootStatus.REMOVING]: 'Quitando…',
  },
  rootFiles: (n) => `${n} archivos`,
  rootChecked: (d) => `Revisada el ${d}`,
  rootNeverChecked: 'Aún no se ha terminado de revisar',
  rescanStarted: 'Se buscarán cambios en segundo plano. Puede seguir buscando mientras tanto.',
  rootAdded: (p) => `Se añadió «${p}». Se está revisando; puede buscar mientras tanto.`,
  remoteWarning: 'Esta carpeta está en un disco de red o extraíble. Si se desconecta, sus archivos se marcarán como «No disponible».',
  bookmarksTitle: 'Marcadores de Firefox',
  bookmarksToggle: 'Incluir marcadores de Firefox',
  bookmarksIntro: 'Busca también entre los marcadores guardados en Firefox. Solo se leen los marcadores, nunca el historial ni las contraseñas, y nada sale de este equipo.',
  bookmarksCount: (n, when) => `${n} marcadores · leídos ${when}`,
  bookmarksState: {
    [ReadState.NEVER]: 'Todavía no se han leído los marcadores.',
    [ReadState.READING]: 'Leyendo los marcadores de Firefox…',
    [ReadState.OK]: '',
    [ReadState.FROM_BACKUP]: 'Firefox no dejó leer sus marcadores más recientes; se usa su última copia de seguridad.',
    [ReadState.NO_FIREFOX]: 'No se han encontrado marcadores de Firefox en este equipo.',
    [ReadState.FAILED]: 'No se pudieron leer los marcadores de Firefox. Se volverá a intentar más tarde.',
  },
  settingsButton: 'Ajustes',
  settingsTitle: 'Ajustes',
  languageLabel: 'Idioma',
  languageOptions: {
    [LanguageChoice.SYSTEM]: 'El del sistema', [LanguageChoice.ES]: 'Español',
    [LanguageChoice.EN]: 'English',
  },
  removeTitle: '¿Quitar esta carpeta?',
  removeBody: 'Dejará de aparecer en las búsquedas. Sus archivos no se borran del disco.',
  cancel: 'Cancelar',
  removeConfirm: 'Quitar carpeta',

  revealedParent: 'Se abrió la carpeta, pero no se pudo seleccionar el archivo.',
  overlapInside: (p) => `Esa carpeta ya está incluida, porque está dentro de «${p}».`,
  overlapParent: (p) => `Esa carpeta contiene «${p}», que ya está en la lista. Quite primero «${p}» y vuelva a añadir la carpeta más grande.`,
  overlapGeneric: 'Esa carpeta ya está en la lista o está dentro de otra de la lista.',

  // ApiError.code -> text, per action. Unknown codes fall back to `generic`.
  errors: {
    generic: 'Ha ocurrido un error. Inténtelo de nuevo.',
    reveal: {
      [ErrorCode.NOT_FOUND]: 'El archivo ya no está en esa ubicación.',
      [ErrorCode.UNAVAILABLE]: 'Esa carpeta no está disponible ahora. Compruebe que el disco está conectado.',
    },
    openBookmark: {
      [ErrorCode.NOT_FOUND]: 'Ese marcador ya no existe en Firefox.',
      [ErrorCode.UNAVAILABLE]: 'Este marcador no es una página web y no se puede abrir.',
    },
    addRoot: {
      [ErrorCode.NOT_FOUND]: 'No se encuentra esa carpeta.',
      [ErrorCode.NOT_A_DIRECTORY]: 'Eso no es una carpeta. Elija una carpeta.',
      [ErrorCode.UNAVAILABLE]: 'No se puede abrir esa carpeta. Compruebe que el disco está conectado.',
    },
    any: {
      [ErrorCode.SCHEMA_TOO_NEW]: 'Los datos de búsqueda los creó una versión más nueva del programa. Instale la versión más reciente.',
    },
  },
  backendMissing: 'No se pudo conectar con el programa. Ciérrelo y vuelva a abrirlo.',
};
