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
/// region; the object is immutable and is the source of truth for its code.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Anchor {
    #[serde(rename = "revision")]
    object_id: String,
    path: String,
    hunk_header: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path_encoding: Option<PathEncoding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
enum PathEncoding {
    GitBytes,
    EscapedUtf8,
}

impl Anchor {
    pub fn new(object_id: impl Into<String>, location: HunkLocation) -> Self {
        let path_encoding = if location.path.starts_with("git-path:bytes:") {
            Some(PathEncoding::GitBytes)
        } else if location.path.starts_with("git-path:utf8:") {
            Some(PathEncoding::EscapedUtf8)
        } else {
            None
        };
        Self {
            object_id: object_id.into(),
            path: location.path,
            hunk_header: location.hunk_header,
            path_encoding,
        }
    }

    pub fn object_id(&self) -> &str {
        &self.object_id
    }

    pub fn location(&self) -> HunkLocation {
        HunkLocation::new(&self.path, &self.hunk_header)
    }

    pub fn decoded_path_bytes(&self) -> Option<Vec<u8>> {
        self.path_encoding
            .and_then(|_| crate::domain::diff::decode_git_path(&self.path))
    }
}

#[cfg(test)]
mod tests {
    use super::{Anchor, HunkLocation};

    #[test]
    fn legacy_marker_like_paths_are_not_reinterpreted() {
        let legacy: Anchor = serde_json::from_str(
            r#"{"revision":"abc","path":"git-path:utf8:foo","hunk_header":"@@ -1 +1 @@"}"#,
        )
        .unwrap();
        let encoded = Anchor::new("abc", HunkLocation::new("git-path:utf8:foo", "@@ -1 +1 @@"));

        assert_eq!(legacy.decoded_path_bytes(), None);
        assert_eq!(encoded.decoded_path_bytes(), Some(b"foo".to_vec()));
    }
}
