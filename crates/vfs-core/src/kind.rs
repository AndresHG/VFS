//! File name extension -> [`FileKind`] (the type filter of VFS-1 AC-11).

use crate::types::FileKind;

const DOCUMENT: &[&str] = &[
    "pdf", "doc", "docx", "odt", "rtf", "txt", "md", "xls", "xlsx", "ods", "csv", "ppt", "pptx",
    "odp", "pages", "numbers", "key", "epub",
];
const PHOTO: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "tif", "tiff", "heic", "heif", "webp", "raw", "cr2", "cr3",
    "nef", "arw", "dng", "orf", "rw2", "psd", "svg",
];
const VIDEO: &[&str] = &[
    "mp4", "mov", "avi", "mkv", "wmv", "m4v", "mpg", "mpeg", "3gp", "webm", "mts", "m2ts", "flv",
    "vob",
];
const AUDIO: &[&str] = &[
    "mp3", "wav", "flac", "aac", "m4a", "ogg", "wma", "aiff", "aif", "opus", "mid", "midi", "amr",
];

/// Kind of a file from the extension of its name, case-insensitively. A name without an
/// extension (or a dot file such as `.profile`) is `Other`.
pub fn file_kind(name: &str) -> FileKind {
    let ext = match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_ascii_lowercase(),
        _ => return FileKind::Other,
    };
    let ext = ext.as_str();
    if DOCUMENT.contains(&ext) {
        FileKind::Document
    } else if PHOTO.contains(&ext) {
        FileKind::Photo
    } else if VIDEO.contains(&ext) {
        FileKind::Video
    } else if AUDIO.contains(&ext) {
        FileKind::Audio
    } else {
        FileKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VFS-1 AC-11: each type filter class is recognised from the extension.
    #[test]
    fn classifies_by_extension() {
        assert_eq!(
            file_kind("Notaría Pérez escritura 2019.pdf"),
            FileKind::Document
        );
        assert_eq!(file_kind("IMG_0001.JPG"), FileKind::Photo);
        assert_eq!(file_kind("boda.MOV"), FileKind::Video);
        assert_eq!(file_kind("tema.flac"), FileKind::Audio);
        assert_eq!(file_kind("setup.exe"), FileKind::Other);
        assert_eq!(file_kind("LEEME"), FileKind::Other);
        assert_eq!(file_kind(".pdf"), FileKind::Other);
        assert_eq!(file_kind("archivo."), FileKind::Other);
    }
}
