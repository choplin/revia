//! Immutable review locations and Git-backed anchor identity.

/// The visible review target within one diff: a file and one hunk header.
///
/// This deliberately excludes a revision. An `Anchor` adds immutable Git
/// provenance to a location; navigation and thread grouping use the location.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HunkLocation {
    path: String,
    hunk_header: String,
}

impl HunkLocation {
    pub fn new(path: impl Into<String>, hunk_header: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            hunk_header: hunk_header.into(),
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn hunk_header(&self) -> &str {
        &self.hunk_header
    }
}

/// A stable location in a Git object. The hunk header identifies the reviewed
/// region; the revision is immutable and is the source of truth for its code.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Anchor {
    revision: String,
    path: String,
    hunk_header: String,
}

impl Anchor {
    pub fn new(revision: impl Into<String>, location: HunkLocation) -> Self {
        Self {
            revision: revision.into(),
            path: location.path,
            hunk_header: location.hunk_header,
        }
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn location(&self) -> HunkLocation {
        HunkLocation::new(&self.path, &self.hunk_header)
    }
}
