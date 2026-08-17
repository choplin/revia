pub(crate) const FILE: &str = "󰈔";
pub(crate) const NEXT: &str = "";
pub(crate) const SEARCH: &str = "";
pub(crate) const SEARCH_CHAR: char = '';
pub(crate) const NEEDS_ATTENTION: &str = "";
pub(crate) const OPEN: &str = "";
pub(crate) const RESOLVED: &str = "";
pub(crate) const SCROLL_THUMB: &str = "▐";
/// Direction-of-change markers for navigation rows, where the exact
/// additions/deletions belong on the file boundary rather than the rail.
///
/// Deliberately plain text rather than the Octicons diff glyphs these would
/// otherwise use: terminals render Private Use Area glyphs two cells wide, and
/// the marker sits against the rail border where that overflows.
pub(crate) const CHANGE_ADDED: &str = "+";
pub(crate) const CHANGE_REMOVED: &str = "-";
pub(crate) const CHANGE_BOTH: &str = "±";
pub(crate) const CHANGE_NONE: &str = " ";
