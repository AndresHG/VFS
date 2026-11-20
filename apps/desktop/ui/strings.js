// Every user-visible string (VFS-1 R9). Spanish, impersonal/usted, plain words: never
// "indexar", "índice" or "reindexar" (VFS-1 decision 16). Functions take already-formatted values.

export const S = {
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
  clearFilters: 'Quitar filtros',
  clearFiltersN: (n) => `Quitar filtros (${n})`,
  rangeInverted: 'La fecha «Desde» es posterior a «Hasta».',

  kindsPlural: {
    document: 'Documentos', photo: 'Fotos', video: 'Vídeos',
    audio: 'Audio', folder: 'Carpetas', other: 'Otros',
  },
  kindSingular: {
    document: 'Documento', photo: 'Foto', video: 'Vídeo',
    audio: 'Audio', folder: 'Carpeta', other: 'Otro',
  },

  dateOptions: {
    any: 'Cualquier fecha', week: 'Últimos 7 días', month: 'Últimos 30 días',
    year: 'Último año', custom: 'Elegir fechas…',
  },
  sizeOptions: {
    any: 'Cualquier tamaño', small: 'Menos de 1 MB', medium: 'De 1 MB a 100 MB',
    large: 'De 100 MB a 1 GB', huge: 'Más de 1 GB',
  },
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
  unavailableBadge: 'No disponible',
  rowLabel: (name, kind, dir, date, size, unavailable) =>
    [name, kind, `en ${dir}`, date && `modificado el ${date}`, size, unavailable && 'no disponible']
      .filter(Boolean).join(', '),
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
    pending: 'En espera', scanning: 'Revisando…', ready: 'Lista',
    unavailable: 'No disponible', removing: 'Quitando…',
  },
  rootFiles: (n) => `${n} archivos`,
  rootChecked: (d) => `Revisada el ${d}`,
  rootNeverChecked: 'Aún no se ha terminado de revisar',
  rescanStarted: 'Se buscarán cambios en segundo plano. Puede seguir buscando mientras tanto.',
  rootAdded: (p) => `Se añadió «${p}». Se está revisando; puede buscar mientras tanto.`,
  remoteWarning: 'Esta carpeta está en un disco de red o extraíble. Si se desconecta, sus archivos se marcarán como «No disponible».',
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
      not_found: 'El archivo ya no está en esa ubicación.',
      unavailable: 'Esa carpeta no está disponible ahora. Compruebe que el disco está conectado.',
    },
    addRoot: {
      not_found: 'No se encuentra esa carpeta.',
      not_a_directory: 'Eso no es una carpeta. Elija una carpeta.',
      unavailable: 'No se puede abrir esa carpeta. Compruebe que el disco está conectado.',
    },
    any: {
      schema_too_new: 'Los datos de búsqueda los creó una versión más nueva del programa. Instale la versión más reciente.',
    },
  },
  backendMissing: 'No se pudo conectar con el programa. Ciérrelo y vuelva a abrirlo.',
};
