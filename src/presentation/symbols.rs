use urushi::Color;

pub(crate) const FILE: &str = "󰈔";
pub(crate) const FOLDER: &str = "";
pub(crate) const SEARCH: &str = "";
pub(crate) const SEARCH_CHAR: char = '';
pub(crate) const SCROLL_THUMB: &str = "▐";
/// Git status markers used by LazyGit in the leading file column.
pub(crate) const CHANGE_ADDED: &str = "A";
pub(crate) const CHANGE_REMOVED: &str = "D";
pub(crate) const CHANGE_BOTH: &str = "M";
pub(crate) const CHANGE_RENAMED: &str = "R";
pub(crate) const CHANGE_COPIED: &str = "C";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileIcon {
    pub(crate) glyph: &'static str,
    pub(crate) color: Color,
}

const fn icon(glyph: &'static str, red: u8, green: u8, blue: u8) -> FileIcon {
    FileIcon {
        glyph,
        color: Color::Rgb(red, green, blue),
    }
}

/// Returns LazyGit's built-in icon and color for the common file types Revia displays.
pub(crate) fn file_type(path: &str) -> FileIcon {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let extension = filename
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    match (filename, extension.as_deref()) {
        ("Cargo.toml" | "Cargo.lock", _) => icon("", 0xde, 0xa5, 0x84),
        (_, Some("rs")) => icon("", 0xff, 0x70, 0x43),
        ("go.mod" | "go.sum", _) | (_, Some("go")) => icon("", 0x02, 0xac, 0xc1),
        (_, Some("js" | "mjs")) => icon("󰌞", 0xff, 0xca, 0x29),
        (_, Some("jsx")) => icon("", 0xff, 0xca, 0x29),
        (_, Some("cjs")) => icon("", 0xcb, 0xcb, 0x41),
        (_, Some("ts")) => icon("󰛦", 0x01, 0x88, 0xd1),
        (_, Some("tsx")) => icon("", 0x04, 0xbc, 0xd4),
        (_, Some("py")) => icon("", 0xfe, 0xd8, 0x36),
        (_, Some("pyi")) => icon("", 0xff, 0xa6, 0x1a),
        (_, Some("json" | "jsonc")) => icon("", 0xfa, 0xa8, 0x25),
        (_, Some("md")) => icon("", 0x42, 0xa5, 0xf5),
        (_, Some("mdx")) => icon("", 0xff, 0xca, 0x29),
        (_, Some("toml")) => icon("", 0x9c, 0x42, 0x21),
        (_, Some("yml" | "yaml")) => icon("", 0xa0, 0x74, 0xb3),
        (_, Some("sh" | "zsh")) => icon("󰆍", 0xff, 0x70, 0x43),
        (_, Some("bash")) => icon("", 0xff, 0x70, 0x43),
        (_, Some("fish")) => icon("󰈺", 0xff, 0x70, 0x43),
        (_, Some("html" | "htm")) => icon("", 0xe4, 0x4e, 0x27),
        (_, Some("css")) => icon("", 0x42, 0xa5, 0xf5),
        (_, Some("scss" | "sass")) => icon("", 0xec, 0x41, 0x7a),
        (_, Some("less")) => icon("", 0x02, 0x77, 0xbd),
        (_, Some("svg")) => icon("󰜡", 0xff, 0xb3, 0x00),
        (_, Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "ico")) => icon("󰈟", 0x25, 0xa6, 0xa0),
        (_, Some("lock")) => icon("", 0xff, 0xd5, 0x50),
        _ => icon(FILE, 0x87, 0x87, 0x87),
    }
}

#[cfg(test)]
mod tests {
    use super::{FILE, file_type, icon};

    #[test]
    fn file_type_icons_follow_filename_and_extension() {
        assert_eq!(file_type("src/main.rs"), icon("", 0xff, 0x70, 0x43));
        assert_eq!(file_type("web/component.tsx"), icon("", 0x04, 0xbc, 0xd4));
        assert_eq!(file_type("README.md"), icon("", 0x42, 0xa5, 0xf5));
        assert_eq!(
            file_type("assets/unknown.data"),
            icon(FILE, 0x87, 0x87, 0x87)
        );
    }
}
