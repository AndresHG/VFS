//! `vfs gen-db`: a deterministic synthetic index shaped like a Spanish personal archive, for
//! the AC-9 benchmark (VFS-1). Every row goes through `vfs_core::db::ingest`, the indexer's own
//! write helpers, so the database has exactly the layout an indexed tree of that shape has.
//!
//! Shape: one root; about one directory per 13 files, up to 18 levels deep (a planted chain
//! reaches 17); long names; accents and ñ, 5 % of names in NFD; dates 2000–2025; sizes
//! log-normal per kind (photos and documents dominate the count, a few videos of 4–60 GB);
//! camera and WhatsApp counters (`IMG_0001`, `DSC00042`, `IMG-20190312-WA0007`), "Copia de …"
//! and "(2)" duplicates; three pathological directories holding 3 %, 2 % and 1 % of all files;
//! empty directories; and canary files at known paths that the benchmark looks for.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::{Connection, Transaction, TransactionBehavior};
use vfs_core::db::{self, ingest};

pub type AnyResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Absolute path of the synthetic root.
pub const ROOT_PATH: &str = "/Volumes/Archivo";
/// Deepest directory level the random tree reaches (the root directory is level 0).
const MAX_DEPTH: u32 = 18;
/// Average files per directory.
const FILES_PER_DIR: u64 = 13;
/// Rows written per transaction.
const BATCH_ROWS: u64 = 50_000;
/// First and last year of file dates.
const FIRST_YEAR: i32 = 2000;
const LAST_YEAR: i32 = 2025;

/// Planted files (directory relative to the root, name), found by the benchmark's canary
/// queries. Inserted first, so they exist even for tiny `--files`.
pub const CANARIES: &[(&str, &str)] = &[
    ("Gestoría/Año 2019", "Notaría Pérez escritura 2019.pdf"),
    (
        "Documentos/Casa",
        "Contrato de alquiler Calle Peñalara 14.pdf",
    ),
    (
        DEEP_CHAIN,
        "Informe pericial Ibáñez definitivo revisado por la gestoría.docx",
    ),
    (
        "Vídeos/2014/Boda Íñigo y Begoña",
        "Boda Íñigo y Begoña ceremonia completa.mp4",
    ),
];

/// A 17-level chain (an old backup copied inside a backup).
const DEEP_CHAIN: &str = "Copia de seguridad portátil/Users/ana/Documents/Disco viejo/\
Backup 2009-03-14/Mis documentos/Trabajo/Clientes/Pérez Ibáñez/2008/Expedientes/\
Expediente 2008-117/Anexos/Escaneos/Originales/Sin clasificar/Revisar";

/// Pathological directories and the share of all files each one holds.
const HUGE_DIRS: &[(&str, f64)] = &[
    ("Fotos del móvil/WhatsApp/Media/WhatsApp Images", 0.03),
    ("Cámara/DCIM/100APPLE", 0.02),
    ("Descargas", 0.01),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Theme {
    Photo,
    Doc,
    Video,
    Audio,
    Mixed,
}

/// Top-level folders, their theme and the weight with which new directories land below them.
const TOP: &[(&str, Theme, u32)] = &[
    ("Fotos", Theme::Photo, 20),
    ("Imágenes", Theme::Photo, 8),
    ("Fotos del móvil", Theme::Photo, 10),
    ("Cámara", Theme::Photo, 7),
    ("Documentos", Theme::Doc, 9),
    ("Gestoría", Theme::Doc, 5),
    ("Trabajo", Theme::Doc, 5),
    ("Papeles de casa", Theme::Doc, 3),
    ("Universidad", Theme::Doc, 2),
    ("Banco", Theme::Doc, 1),
    ("Vídeos", Theme::Video, 6),
    ("Películas caseras", Theme::Video, 2),
    ("Música", Theme::Audio, 6),
    ("Notas de voz", Theme::Audio, 2),
    ("Escritorio antiguo", Theme::Mixed, 4),
    ("Copia de seguridad portátil", Theme::Mixed, 4),
    ("Descargas", Theme::Mixed, 2),
    ("Disco duro viejo", Theme::Mixed, 2),
    ("Familia", Theme::Mixed, 2),
];

const FIRST: &[&str] = &[
    "Begoña",
    "Íñigo",
    "María",
    "José",
    "Lucía",
    "Álvaro",
    "Pilar",
    "Jesús",
    "Mónica",
    "Raúl",
    "Ana",
    "Martín",
    "Sofía",
    "Andrés",
    "Nuria",
    "Ramón",
    "Concepción",
    "Iñaki",
    "Rocío",
    "Óscar",
    "Carmen",
    "Javier",
];
const SURNAME: &[&str] = &[
    "Pérez",
    "García",
    "Martínez",
    "López",
    "Sánchez",
    "Gómez",
    "Fernández",
    "Ruiz",
    "Díaz",
    "Muñoz",
    "Álvarez",
    "Jiménez",
    "Hernández",
    "Peña",
    "Ibáñez",
    "Núñez",
    "Castaño",
    "Ordóñez",
];
const PLACE: &[&str] = &[
    "Cádiz",
    "Málaga",
    "León",
    "Ávila",
    "Córdoba",
    "Logroño",
    "Peñíscola",
    "A Coruña",
    "San Sebastián",
    "Cáceres",
    "Jaén",
    "Almería",
    "Benidorm",
    "Tenerife",
    "París",
    "Lisboa",
    "Roma",
    "Nueva York",
];
const MONTH: &[&str] = &[
    "Enero",
    "Febrero",
    "Marzo",
    "Abril",
    "Mayo",
    "Junio",
    "Julio",
    "Agosto",
    "Septiembre",
    "Octubre",
    "Noviembre",
    "Diciembre",
];
const DOC_SUBJECT: &[&str] = &[
    "Factura Iberdrola",
    "Factura Movistar",
    "Factura Endesa",
    "Recibo comunidad",
    "Nómina",
    "Declaración de la renta",
    "Escritura notaría",
    "Contrato de alquiler",
    "Contrato de trabajo",
    "Presupuesto reforma cocina",
    "Seguro del coche",
    "Póliza hogar Mapfre",
    "Certificado de empadronamiento",
    "Informe médico",
    "Análisis de sangre",
    "Hipoteca",
    "Extracto bancario",
    "Matrícula universidad",
    "Currículum",
    "Acta de la reunión",
    "Libro de familia",
    "Testamento",
    "Recibo IBI",
    "Multa de tráfico",
    "Garantía lavadora",
    "Manual de instrucciones",
    "Apuntes de matemáticas",
    "Trabajo de fin de grado",
    "Receta de la abuela",
    "Menú de Navidad",
];
const DOC_SUFFIX: &[&str] = &[
    "",
    "",
    "",
    " v2",
    " definitivo",
    " FINAL",
    " (firmado)",
    " escaneado",
    " borrador",
    " corregido",
    " - copia",
];
const SONG: &[&str] = &[
    "Corazón contento",
    "La flaca",
    "Mediterráneo",
    "Bailando",
    "Entre dos aguas",
    "Lágrimas negras",
    "Sevillanas del adiós",
    "Canción de cuna",
    "Mi niña bonita",
    "Rumba de la mañana",
];
const ARTIST: &[&str] = &[
    "Serrat",
    "Camarón",
    "Rocío Jurado",
    "Mecano",
    "Estopa",
    "Jarabe de Palo",
    "Paco de Lucía",
    "Los Chunguitos",
];

/// xorshift64* seeded through splitmix64: deterministic, no dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Rng((z ^ (z >> 31)) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n` (`n > 0`).
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    fn range(&mut self, lo: i64, hi_inclusive: i64) -> i64 {
        lo + self.below((hi_inclusive - lo + 1) as u64) as i64
    }

    /// Uniform in `[0, 1)`.
    fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn chance(&mut self, p: f64) -> bool {
        self.f64() < p
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }

    /// Standard normal (Box–Muller).
    fn normal(&mut self) -> f64 {
        let u1 = self.f64().max(1e-12);
        let u2 = self.f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    fn log_normal(&mut self, median: f64, sigma: f64) -> f64 {
        median * (sigma * self.normal()).exp()
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(if m <= 2 { y - 1 } else { y });
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// (year, month, day) of a unix time in UTC.
pub fn civil_from_unix(secs: i64) -> (i32, u32, u32) {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + i64::from(m <= 2)) as i32;
    (y, m, d)
}

/// Rewrites the accented letters this generator uses into base letter + combining mark (NFD).
fn to_nfd(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        let (base, mark) = match c {
            'á' => ('a', '\u{301}'),
            'é' => ('e', '\u{301}'),
            'í' => ('i', '\u{301}'),
            'ó' => ('o', '\u{301}'),
            'ú' => ('u', '\u{301}'),
            'Á' => ('A', '\u{301}'),
            'É' => ('E', '\u{301}'),
            'Í' => ('I', '\u{301}'),
            'Ó' => ('O', '\u{301}'),
            'Ú' => ('U', '\u{301}'),
            'ñ' => ('n', '\u{303}'),
            'Ñ' => ('N', '\u{303}'),
            'ü' => ('u', '\u{308}'),
            _ => {
                out.push(c);
                continue;
            }
        };
        out.push(base);
        out.push(mark);
    }
    out
}

struct DirInfo {
    id: i64,
    rel: String,
    depth: u32,
    theme: Theme,
    year: i32,
    /// A year already appears in this directory's path.
    dated: bool,
}

/// What `gen-db` wrote.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenReport {
    pub files: u64,
    pub dirs: u64,
    pub max_depth: u32,
    pub mean_depth: f64,
    pub largest_dir_files: u64,
    pub empty_dirs: u64,
    /// Files per kind code (`FileKind::code`), index 0..=4.
    pub files_by_kind: [u64; 5],
    pub db_bytes: u64,
    pub elapsed_ms: u64,
}

struct Gen<'c> {
    conn: &'c Connection,
    tx: Option<Transaction<'c>>,
    rows: u64,
    rng: Rng,
    root_id: i64,
    dirs: Vec<DirInfo>,
    children: HashMap<(usize, String), usize>,
    /// Directory indexes per theme (`Theme as usize`), root directory excluded.
    by_theme: [Vec<usize>; 5],
    progress: bool,
}

impl<'c> Gen<'c> {
    fn tx(&self) -> &Transaction<'c> {
        self.tx.as_ref().expect("transaction open")
    }

    /// Counts one written row and commits every `BATCH_ROWS`.
    fn wrote(&mut self) -> AnyResult<()> {
        self.rows += 1;
        if self.rows.is_multiple_of(BATCH_ROWS) {
            self.tx.take().expect("transaction open").commit()?;
            self.tx = Some(Transaction::new_unchecked(
                self.conn,
                TransactionBehavior::Immediate,
            )?);
        }
        Ok(())
    }

    fn mtime_in(&mut self, year: i32) -> i64 {
        let month = self.rng.range(1, 12) as u32;
        let day = self.rng.range(1, 28) as u32;
        days_from_civil(year, month, day) * 86_400 + self.rng.range(0, 86_399)
    }

    fn random_year(&mut self) -> i32 {
        self.rng.range(i64::from(FIRST_YEAR), i64::from(LAST_YEAR)) as i32
    }

    /// Creates the child `name` of `parent` (or returns the existing one).
    fn child(
        &mut self,
        parent: usize,
        name: &str,
        theme: Theme,
        year: Option<i32>,
    ) -> AnyResult<usize> {
        if let Some(&idx) = self.children.get(&(parent, name.to_string())) {
            return Ok(idx);
        }
        let p = &self.dirs[parent];
        let rel = if p.rel.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", p.rel, name)
        };
        let (pid, depth, pdated, pyear) = (p.id, p.depth + 1, p.dated, p.year);
        let year_named = year.is_some();
        let year = match year {
            Some(y) => y,
            None if depth > 1 => pyear,
            None => self.random_year(),
        };
        let mtime = self.mtime_in(year);
        let id = ingest::insert_dir(self.tx(), self.root_id, pid, &rel, mtime)?;
        ingest::set_dir_state(self.tx(), id, ingest::ScanState::Clean)?;
        self.wrote()?;
        self.dirs.push(DirInfo {
            id,
            rel,
            depth,
            theme,
            year,
            dated: pdated || year_named,
        });
        let idx = self.dirs.len() - 1;
        self.children.insert((parent, name.to_string()), idx);
        self.by_theme[theme as usize].push(idx);
        Ok(idx)
    }

    /// Creates every missing component of a '/'-separated path below the root directory.
    fn ensure_path(&mut self, rel: &str) -> AnyResult<usize> {
        let mut cur = 0;
        for comp in rel.split('/') {
            let theme = if cur == 0 {
                TOP.iter()
                    .find(|t| t.0 == comp)
                    .map_or(Theme::Mixed, |t| t.1)
            } else {
                self.dirs[cur].theme
            };
            let year = comp
                .rsplit(' ')
                .next()
                .and_then(|w| w.parse::<i32>().ok())
                .filter(|y| (FIRST_YEAR..=LAST_YEAR).contains(y));
            cur = self.child(cur, comp, theme, year)?;
        }
        Ok(cur)
    }

    fn event(&mut self) -> String {
        let r = self.rng.below(13);
        let f = *self.rng.pick(FIRST);
        let p = *self.rng.pick(PLACE);
        match r {
            0 => format!("Cumpleaños de {f}"),
            1 => format!("Boda de {f} y {}", self.rng.pick(FIRST)),
            2 => format!("Bautizo de {f}"),
            3 => format!("Comunión de {f}"),
            4 => "Navidad".to_string(),
            5 => "Nochevieja".to_string(),
            6 => "Semana Santa".to_string(),
            7 => format!("Viaje a {p}"),
            8 => format!("Vacaciones en {p}"),
            9 => format!("Fin de semana en {p}"),
            10 => format!("Excursión a {p}"),
            11 => format!("Fiesta de {f}"),
            _ => format!("Graduación de {f}"),
        }
    }

    /// A random directory name below `parent`; returns (name, year if the name carries one).
    fn dir_name(&mut self, parent: usize) -> (String, Option<i32>) {
        let (theme, dated, pyear) = {
            let p = &self.dirs[parent];
            (p.theme, p.dated, p.year)
        };
        let r = self.rng.f64();
        if !dated && r < 0.35 {
            let y = self.random_year();
            let name = match self.rng.below(4) {
                0 => format!("{y}"),
                1 => format!("Año {y}"),
                2 => format!("{} {y}", TOP_LABEL[self.rng.below(3) as usize]),
                _ => format!("{y} - {}", self.event()),
            };
            return (name, Some(y));
        }
        if dated && r < 0.25 {
            let m = self.rng.below(12) as usize;
            let name = if self.rng.chance(0.5) {
                format!("{:02} - {}", m + 1, MONTH[m])
            } else {
                format!("{} {pyear}", MONTH[m])
            };
            return (name, None);
        }
        let name = if r < 0.6 {
            self.event()
        } else if r < 0.67 {
            format!("Copia de {}", self.event())
        } else if r < 0.73 {
            match self.rng.below(3) {
                0 => "Nueva carpeta".to_string(),
                1 => format!("Nueva carpeta ({})", self.rng.range(2, 9)),
                _ => "Sin título".to_string(),
            }
        } else if r < 0.88 {
            self.themed_dir_name(theme)
        } else if r < 0.93 {
            format!(
                "{} con {} {} y {} {} ({}) originales sin editar ni recortar",
                self.event(),
                self.rng.pick(FIRST),
                self.rng.pick(SURNAME),
                self.rng.pick(FIRST),
                self.rng.pick(SURNAME),
                pyear
            )
        } else {
            format!("{} {}", self.rng.pick(FIRST), self.rng.pick(SURNAME))
        };
        (name, None)
    }

    fn themed_dir_name(&mut self, theme: Theme) -> String {
        match theme {
            Theme::Doc => match self.rng.below(3) {
                0 => format!(
                    "Cliente {} {}",
                    self.rng.pick(SURNAME),
                    self.rng.pick(SURNAME)
                ),
                1 => format!(
                    "Expediente {}-{:03}",
                    self.random_year(),
                    self.rng.range(1, 400)
                ),
                _ => self.rng.pick(DOC_SUBJECT).to_string(),
            },
            Theme::Photo => self
                .rng
                .pick(&["Selección", "Editadas", "Originales", "DCIM", "100CANON"])
                .to_string(),
            Theme::Video => self
                .rng
                .pick(&["Montajes", "Brutos", "Para DVD"])
                .to_string(),
            Theme::Audio => format!("{} - Grandes éxitos", self.rng.pick(ARTIST)),
            Theme::Mixed => self
                .rng
                .pick(&["Varios", "Cosas", "Importante", "Pendiente", "Viejo"])
                .to_string(),
        }
    }

    fn random_dirs(&mut self, count: u64) -> AnyResult<()> {
        let weights: u32 = TOP.iter().map(|t| t.2).sum();
        for _ in 0..count {
            // A top-level folder by weight fixes the theme, so the kind mix is stable; the
            // parent is then a recent directory of that theme (deep chains), the top-level
            // folder itself, or any directory of that theme.
            let mut w = self.rng.below(u64::from(weights)) as u32;
            let top = TOP
                .iter()
                .find(|t| {
                    if w < t.2 {
                        true
                    } else {
                        w -= t.2;
                        false
                    }
                })
                .expect("weights cover the range");
            let top_idx = self.children[&(0, top.0.to_string())];
            let parent = loop {
                let list = &self.by_theme[top.1 as usize];
                let n = list.len();
                let idx = if n > 9 && self.rng.chance(0.4) {
                    list[n - 1 - self.rng.below(8) as usize]
                } else if self.rng.chance(0.15) {
                    top_idx
                } else {
                    list[self.rng.below(n as u64) as usize]
                };
                if self.dirs[idx].depth < MAX_DEPTH {
                    break idx;
                }
            };
            let theme = self.dirs[parent].theme;
            let (mut name, year) = self.dir_name(parent);
            let mut k = 2;
            let base = name.clone();
            while self.children.contains_key(&(parent, name.clone())) {
                name = format!("{base} ({k})");
                k += 1;
            }
            self.child(parent, &name, theme, year)?;
        }
        Ok(())
    }
}

const TOP_LABEL: &[&str] = &["Fotos", "Vacaciones", "Papeles"];

/// Kind of a file in a directory of `theme`, as a `FileKind` code (0 other .. 4 audio).
fn pick_kind(rng: &mut Rng, theme: Theme) -> i64 {
    // photo, document, video, audio, other (percent)
    let mix: [u64; 5] = match theme {
        Theme::Photo => [82, 3, 10, 1, 4],
        Theme::Doc => [8, 78, 1, 1, 12],
        Theme::Video => [15, 2, 75, 3, 5],
        Theme::Audio => [2, 3, 2, 88, 5],
        Theme::Mixed => [35, 30, 10, 10, 15],
    };
    let mut r = rng.below(100);
    for (i, w) in mix.iter().enumerate() {
        if r < *w {
            return [2, 1, 3, 4, 0][i];
        }
        r -= w;
    }
    0
}

/// Per-directory name counters (camera numbering continues within a folder).
struct Counters {
    img: u32,
    dsc: u32,
    mov: u32,
    voice: u32,
}

/// (base name, extension, size in bytes) of a random file of `kind` dated `mtime`.
fn file_name(rng: &mut Rng, kind: i64, mtime: i64, c: &mut Counters) -> (String, String, u64) {
    let (y, m, d) = civil_from_unix(mtime);
    let hms = mtime.rem_euclid(86_400);
    let (hh, mi, ss) = (hms / 3600, hms / 60 % 60, hms % 60);
    let mb = 1024.0 * 1024.0;
    match kind {
        2 => {
            let r = rng.f64();
            let (base, ext, size) = if r < 0.35 {
                c.img = c.img % 9999 + 1;
                let ext = *rng.pick(&["jpg", "JPG", "JPG", "JPG", "HEIC", "HEIC", "jpeg", "PNG"]);
                (
                    format!("IMG_{:04}", c.img),
                    ext,
                    rng.log_normal(2.5 * mb, 0.5),
                )
            } else if r < 0.5 {
                c.dsc = c.dsc % 99_999 + 1;
                let ext = *rng.pick(&["JPG", "JPG", "JPG", "NEF", "ARW"]);
                let raw = if ext == "JPG" { 1.0 } else { 8.0 };
                let size = rng.log_normal(4.0 * mb * raw, 0.4);
                (format!("DSC{:05}", c.dsc), ext, size)
            } else if r < 0.62 {
                let n = rng.range(1, 9999);
                let base = format!("IMG-{y}{m:02}{d:02}-WA{n:04}");
                (base, "jpg", rng.log_normal(0.2 * mb, 0.5))
            } else if r < 0.72 {
                let base = format!("{y}{m:02}{d:02}_{hh:02}{mi:02}{ss:02}");
                (base, "jpg", rng.log_normal(3.0 * mb, 0.4))
            } else if r < 0.8 {
                let base =
                    format!("Captura de pantalla {y}-{m:02}-{d:02} a las {hh}.{mi:02}.{ss:02}");
                (base, "png", rng.log_normal(0.4 * mb, 0.6))
            } else {
                let base = match rng.below(3) {
                    0 => format!("{} en {}", rng.pick(FIRST), rng.pick(PLACE)),
                    1 => format!("Foto de {} y {}", rng.pick(FIRST), rng.pick(FIRST)),
                    _ => format!("{} {} {}", rng.pick(PLACE), y, rng.range(1, 300)),
                };
                (base, "jpg", rng.log_normal(2.0 * mb, 0.6))
            };
            let base = if rng.chance(0.06) {
                format!("Copia de {base}")
            } else if rng.chance(0.03) {
                format!("{base}-editada")
            } else {
                base
            };
            (base, ext.to_string(), size as u64)
        }
        1 => {
            let subject = *rng.pick(DOC_SUBJECT);
            let base = match rng.below(4) {
                0 => format!("{subject} {} {y}", MONTH[(m - 1) as usize].to_lowercase()),
                1 => format!("{subject} {y}"),
                2 => format!("{subject} {} {}", rng.pick(FIRST), rng.pick(SURNAME)),
                _ => format!("{y}-{m:02}-{d:02} {subject}"),
            };
            let mut base = format!("{base}{}", rng.pick(DOC_SUFFIX));
            if rng.chance(0.08) {
                base = format!("Copia de {base}");
            }
            if rng.chance(0.05) {
                base = format!(
                    "{base} - {} {} {} - expediente {} - documentación completa para la gestoría revisada",
                    rng.pick(FIRST),
                    rng.pick(SURNAME),
                    rng.pick(SURNAME),
                    rng.range(100, 9999)
                );
            }
            let ext = *rng.pick(&[
                "pdf", "pdf", "pdf", "pdf", "pdf", "pdf", "docx", "docx", "docx", "doc", "doc",
                "xlsx", "xlsx", "xls", "odt", "txt", "pptx",
            ]);
            (base, ext.to_string(), rng.log_normal(150_000.0, 1.4) as u64)
        }
        3 => {
            let (base, ext) = match rng.below(20) {
                0..=7 => (format!("VID_{y}{m:02}{d:02}_{hh:02}{mi:02}{ss:02}"), "mp4"),
                8..=11 => {
                    c.mov = c.mov % 9999 + 1;
                    (format!("MOV_{:04}", c.mov), "MOV")
                }
                12..=16 => {
                    let ev = *rng.pick(&["Boda", "Cumpleaños", "Comunión", "Viaje", "Navidad"]);
                    let ext = *rng.pick(&["mp4", "avi", "mkv", "mp4"]);
                    (format!("{ev} {y} parte {}", rng.range(1, 6)), ext)
                }
                _ => {
                    c.mov = c.mov % 9999 + 1;
                    (format!("{:05}", c.mov), "MTS")
                }
            };
            let size = if rng.chance(0.03) {
                rng.range(4 << 30, 60 << 30) as f64
            } else {
                rng.log_normal(250.0 * mb, 1.3)
            };
            (base, ext.to_string(), size as u64)
        }
        4 => {
            let (base, ext) = match rng.below(4) {
                0 => (
                    format!("{:02} - {}", rng.range(1, 18), rng.pick(SONG)),
                    "mp3",
                ),
                1 => {
                    c.voice += 1;
                    (format!("Nota de voz {}", c.voice), "m4a")
                }
                2 => (format!("Grabación {y}-{m:02}-{d:02} {hh:02}{mi:02}"), "wav"),
                _ => (format!("{} - {}", rng.pick(ARTIST), rng.pick(SONG)), "flac"),
            };
            (base, ext.to_string(), rng.log_normal(4.0 * mb, 0.8) as u64)
        }
        _ => {
            let (base, ext, size) = match rng.below(10) {
                0 => ("Thumbs".to_string(), "db", rng.log_normal(40_000.0, 1.0)),
                1 => ("desktop".to_string(), "ini", 282.0),
                2 => (
                    rng.pick(DOC_SUBJECT).to_string(),
                    "zip",
                    rng.log_normal(5.0 * mb, 1.5),
                ),
                3 => (
                    format!(
                        "Instalador {}",
                        rng.pick(&["Office", "Antivirus", "Impresora HP"])
                    ),
                    "exe",
                    rng.log_normal(80.0 * mb, 1.0),
                ),
                4 => (
                    format!("{} {y}", rng.pick(SURNAME)),
                    "rar",
                    rng.log_normal(20.0 * mb, 1.5),
                ),
                5 => (
                    format!("Copia de seguridad {y}-{m:02}-{d:02}"),
                    "bak",
                    rng.log_normal(300.0 * mb, 1.2),
                ),
                6 => (
                    format!("Disco {}", rng.pick(PLACE)),
                    "iso",
                    rng.range(1 << 30, 8 << 30) as f64,
                ),
                7 => (
                    format!("Acceso directo a {}", rng.pick(DOC_SUBJECT)),
                    "lnk",
                    1500.0,
                ),
                8 => ("LEEME".to_string(), "", 2_000.0),
                _ => (
                    format!("{} {}", rng.pick(DOC_SUBJECT), rng.range(1, 99)),
                    "torrent",
                    30_000.0,
                ),
            };
            (base, ext.to_string(), size as u64)
        }
    }
}

/// Adds " (k)" before the extension until the name is unused in the directory.
fn unique_name(used: &mut HashSet<String>, base: &str, ext: &str) -> String {
    let join = |b: &str| {
        if ext.is_empty() {
            b.to_string()
        } else {
            format!("{b}.{ext}")
        }
    };
    let mut name = join(base);
    let mut k = 2;
    while used.contains(&name) {
        name = join(&format!("{base} ({k})"));
        k += 1;
    }
    used.insert(name.clone());
    name
}

fn db_bytes(path: &Path) -> u64 {
    ["", "-wal"]
        .iter()
        .filter_map(|s| {
            let mut p = path.as_os_str().to_owned();
            p.push(s);
            std::fs::metadata(p).ok()
        })
        .map(|m| m.len())
        .sum()
}

/// Writes a synthetic index of exactly `files` files to `out`, which must not exist yet.
/// Deterministic: the same `files` and `seed` give the same rows (VFS-1 AC-9 harness).
pub fn gen_db(out: &Path, files: u64, seed: u64, progress: bool) -> AnyResult<GenReport> {
    if out.exists() {
        return Err(format!("{} already exists", out.display()).into());
    }
    let started = Instant::now();
    let conn = db::open_rw(out)?;
    let tx = Transaction::new_unchecked(&conn, TransactionBehavior::Immediate)?;
    let root_id = ingest::insert_root(&tx, "/", ROOT_PATH)?;
    let root_dir: i64 = tx.query_row(
        "SELECT id FROM dirs WHERE root_id = ?1 AND rel_path = ''",
        [root_id],
        |r| r.get(0),
    )?;
    ingest::set_dir_state(&tx, root_dir, ingest::ScanState::Clean)?;
    let mut g = Gen {
        conn: &conn,
        tx: Some(tx),
        rows: 0,
        rng: Rng::new(seed),
        root_id,
        dirs: vec![DirInfo {
            id: root_dir,
            rel: String::new(),
            depth: 0,
            theme: Theme::Mixed,
            year: LAST_YEAR,
            dated: false,
        }],
        children: HashMap::new(),
        by_theme: Default::default(),
        progress,
    };
    let mtime = g.mtime_in(LAST_YEAR);
    ingest::set_dir_mtime(g.tx(), root_dir, mtime)?;

    // Fixed structure: top-level folders, canary folders, the deep chain, huge folders.
    for (name, theme, _) in TOP {
        g.child(0, name, *theme, None)?;
    }
    let canaries: Vec<(usize, &str)> = CANARIES
        .iter()
        .take(files as usize)
        .map(|(dir, name)| g.ensure_path(dir).map(|i| (i, *name)))
        .collect::<AnyResult<_>>()?;
    let huge: Vec<(usize, u64)> = HUGE_DIRS
        .iter()
        .map(|(dir, share)| {
            g.ensure_path(dir)
                .map(|i| (i, (files as f64 * share) as u64))
        })
        .collect::<AnyResult<_>>()?;
    let target_dirs = (files / FILES_PER_DIR).max(40);
    let fixed = g.dirs.len() as u64;
    g.random_dirs(target_dirs.saturating_sub(fixed))?;

    // Files per directory: log-normal weights over every directory but the huge ones.
    let mut counts = vec![0u64; g.dirs.len()];
    let huge_total: u64 = huge.iter().map(|h| h.1).sum();
    let mut left = files.saturating_sub(canaries.len() as u64);
    for &(i, n) in &huge {
        let n = n.min(left);
        counts[i] = n;
        left -= n;
    }
    let regular: Vec<usize> = (0..g.dirs.len())
        .filter(|i| !huge.iter().any(|h| h.0 == *i))
        .collect();
    let weights: Vec<f64> = regular.iter().map(|_| g.rng.log_normal(1.0, 1.2)).collect();
    let sum: f64 = weights.iter().sum();
    let mut assigned = 0;
    for (&i, w) in regular.iter().zip(&weights) {
        let n = (left as f64 * w / sum) as u64;
        counts[i] = n;
        assigned += n;
    }
    for _ in assigned..left {
        let i = regular[g.rng.below(regular.len() as u64) as usize];
        counts[i] += 1;
    }
    debug_assert_eq!(
        counts.iter().sum::<u64>() + canaries.len() as u64,
        files,
        "huge {huge_total}"
    );

    let mut by_kind = [0u64; 5];
    let mut used = HashSet::new();
    let mut written = 0u64;
    let tick = Instant::now();
    for (i, &count) in counts.iter().enumerate() {
        used.clear();
        let (dir_id, theme, year) = (g.dirs[i].id, g.dirs[i].theme, g.dirs[i].year);
        for &(ci, name) in canaries.iter().filter(|c| c.0 == i) {
            used.insert(name.to_string());
            let kind = vfs_core::file_kind(name).code();
            let mtime = g.mtime_in(2019);
            let size = if kind == 3 { 38 << 30 } else { 245_760 };
            let file = ingest::NewFile {
                name,
                size,
                mtime,
                lossy: false,
            };
            ingest::insert_file(g.tx(), g.dirs[ci].id, &file)?;
            by_kind[kind as usize] += 1;
            g.wrote()?;
        }
        let mut c = Counters {
            img: g.rng.range(0, 9000) as u32,
            dsc: g.rng.range(0, 90_000) as u32,
            mov: g.rng.range(0, 9000) as u32,
            voice: 0,
        };
        for _ in 0..count {
            let kind = pick_kind(&mut g.rng, theme);
            let y = if g.rng.chance(0.85) {
                year
            } else {
                g.random_year()
            };
            let mtime = g.mtime_in(y);
            let (base, ext, size) = file_name(&mut g.rng, kind, mtime, &mut c);
            let base = if g.rng.chance(0.05) {
                to_nfd(&base)
            } else {
                base
            };
            let name = unique_name(&mut used, &base, &ext);
            let file = ingest::NewFile {
                name: &name,
                size: size.max(1),
                mtime,
                lossy: false,
            };
            ingest::insert_file(g.tx(), dir_id, &file)?;
            by_kind[vfs_core::file_kind(&name).code() as usize] += 1;
            g.wrote()?;
            written += 1;
            if g.progress && written.is_multiple_of(250_000) {
                eprintln!(
                    "  {written} files ({:.0} files/s)",
                    written as f64 / tick.elapsed().as_secs_f64()
                );
            }
        }
    }

    // Root bookkeeping, as the indexer leaves it after a finished first scan (no helper).
    let n_dirs = g.dirs.len() as u64;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64;
    g.tx().execute(
        "UPDATE roots SET status = ?5, files_count = ?2, dirs_count = ?3, last_full_scan_at = ?4 \
         WHERE id = ?1",
        (
            root_id,
            files as i64,
            n_dirs as i64,
            now,
            vfs_core::RootStatus::Ready.code(),
        ),
    )?;
    g.tx.take().expect("transaction open").commit()?;
    let _: (i64, i64, i64) = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;

    let max_depth = g.dirs.iter().map(|d| d.depth).max().unwrap_or(0);
    let mean_depth = g.dirs.iter().map(|d| f64::from(d.depth)).sum::<f64>() / n_dirs as f64;
    let mut per_dir = counts.clone();
    for (ci, _) in &canaries {
        per_dir[*ci] += 1;
    }
    let report = GenReport {
        files,
        dirs: n_dirs,
        max_depth,
        mean_depth,
        largest_dir_files: per_dir.iter().copied().max().unwrap_or(0),
        empty_dirs: per_dir.iter().filter(|n| **n == 0).count() as u64,
        files_by_kind: by_kind,
        db_bytes: 0,
        elapsed_ms: 0,
    };
    drop(g);
    drop(conn);
    Ok(GenReport {
        db_bytes: db_bytes(out),
        elapsed_ms: started.elapsed().as_millis() as u64,
        ..report
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    type Row = (String, String, i64, i64, i64);

    fn rows(path: &Path) -> (Vec<String>, Vec<Row>) {
        let conn = db::open_ro(path).unwrap();
        let mut stmt = conn
            .prepare("SELECT rel_path FROM dirs ORDER BY id")
            .unwrap();
        let dirs = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<String>>>()
            .unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT d.rel_path, f.name, f.size, f.mtime, f.kind FROM files f \
                 JOIN dirs d ON d.id = f.dir_id ORDER BY f.id",
            )
            .unwrap();
        let files = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<Row>>>()
            .unwrap();
        (dirs, files)
    }

    /// VFS-1 AC-9: the benchmark database is reproducible — the same seed gives identical
    /// directories and files, another seed gives different ones.
    #[test]
    fn same_seed_same_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.db");
        let b = tmp.path().join("b.db");
        let c = tmp.path().join("c.db");
        gen_db(&a, 3000, 7, false).unwrap();
        gen_db(&b, 3000, 7, false).unwrap();
        gen_db(&c, 3000, 8, false).unwrap();
        let (ra, rb, rc) = (rows(&a), rows(&b), rows(&c));
        assert_eq!(ra.1.len(), 3000);
        assert_eq!(ra, rb);
        assert_ne!(ra.1, rc.1);
    }

    /// VFS-1 AC-9: `--files` is exact even below the number of planted rows, and an
    /// existing output is never overwritten.
    #[test]
    fn exact_count_and_no_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        for n in [0, 3, 10, 777] {
            let p = tmp.path().join(format!("n{n}.db"));
            let r = gen_db(&p, n, 1, false).unwrap();
            assert_eq!(r.files, n);
            assert_eq!(rows(&p).1.len() as u64, n);
        }
        assert!(gen_db(&tmp.path().join("n3.db"), 3, 1, false).is_err());
    }

    /// VFS-1 AC-9: the synthetic archive has the shape the latency budget is about — deep
    /// paths, long Spanish names with accents (some NFD), camera and copy near-duplicates,
    /// 25+ years of dates, photo/document-dominated counts with some huge videos, empty
    /// folders, pathological directories, and the canaries at their paths.
    #[test]
    fn archive_shape() {
        const N: u64 = 20_000;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("shape.db");
        let report = gen_db(&path, N, 42, false).unwrap();
        let (dirs, files) = rows(&path);
        let n = files.len() as f64;
        assert_eq!(files.len() as u64, N);
        assert_eq!(report.dirs, dirs.len() as u64);
        assert!(
            dirs.len() as u64 >= N / FILES_PER_DIR,
            "{} dirs",
            dirs.len()
        );

        let depth = |rel: &str| rel.split('/').filter(|c| !c.is_empty()).count();
        assert!(dirs.iter().map(|d| depth(d)).max().unwrap() >= 15);
        assert!(report.max_depth <= MAX_DEPTH);
        assert!(dirs.iter().any(|d| d.chars().count() >= 200));
        assert!(files.iter().any(|f| f.1.chars().count() >= 100));

        let share =
            |pred: &dyn Fn(&Row) -> bool| files.iter().filter(|f| pred(f)).count() as f64 / n;
        let kind = |k: i64| share(&|f: &Row| f.4 == k);
        let (photo, doc, video, audio, other) = (kind(2), kind(1), kind(3), kind(4), kind(0));
        assert!((0.35..0.55).contains(&photo), "photo {photo}");
        assert!((0.18..0.34).contains(&doc), "document {doc}");
        assert!((0.06..0.16).contains(&video), "video {video}");
        assert!((0.05..0.14).contains(&audio), "audio {audio}");
        assert!((0.04..0.14).contains(&other), "other {other}");
        assert_eq!(
            report.files_by_kind.iter().sum::<u64>(),
            N,
            "{:?}",
            report.files_by_kind
        );

        let accented = share(&|f: &Row| !f.1.is_ascii());
        assert!(accented > 0.1, "accented {accented}");
        assert!(files.iter().any(|f| f.1.contains('ñ')));
        let nfd = share(&|f: &Row| f.1.contains('\u{301}') || f.1.contains('\u{303}'));
        assert!(nfd > 0.0 && nfd < 0.05, "nfd {nfd}");
        assert!(share(&|f: &Row| f.1.starts_with("IMG_")) > 0.1);
        assert!(share(&|f: &Row| f.1.starts_with("Copia de ")) > 0.01);
        assert!(files.iter().any(|f| f.1.contains(" (2)")));

        let years: Vec<i32> = files.iter().map(|f| civil_from_unix(f.3).0).collect();
        assert_eq!(*years.iter().min().unwrap(), FIRST_YEAR);
        assert_eq!(*years.iter().max().unwrap(), LAST_YEAR);

        assert!(files.iter().any(|f| f.4 == 3 && f.2 > 4 << 30));
        let mut sizes: Vec<i64> = files.iter().map(|f| f.2).collect();
        sizes.sort_unstable();
        let median = sizes[sizes.len() / 2];
        assert!((100_000..10_000_000).contains(&median), "median {median}");

        assert!(report.largest_dir_files as f64 >= 0.025 * n);
        assert!(report.empty_dirs > 0);
        for (dir, name) in CANARIES {
            assert!(
                files.iter().any(|f| f.0 == *dir && f.1 == *name),
                "canary {dir}/{name}"
            );
        }
    }

    /// VFS-1 AC-11: the date helpers round-trip (the CLI's --from/--to and the generator).
    #[test]
    fn civil_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        for secs in [0, 951_782_400, 1_234_567_890, 1_767_225_599] {
            let (y, m, d) = civil_from_unix(secs);
            assert_eq!(days_from_civil(y, m, d), secs.div_euclid(86_400));
        }
        assert_eq!(to_nfd("Peña Á"), "Pen\u{303}a A\u{301}");
    }
}
