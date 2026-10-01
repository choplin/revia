use std::{
    fs,
    io::{self, Read},
    path::Path,
};

use anyhow::{Context, Result};

use crate::domain::diff::{
    DiffDocument, DiffProvenance, DiffRequest, DiffSource, LoadedDiff, PatchInput,
};

use super::git::diff as git_diff;

pub fn load(repository: &Path, request: &DiffRequest) -> Result<LoadedDiff> {
    match &request.source {
        DiffSource::Git(comparison) => {
            git_diff::load(repository, comparison, request.context_lines)
        }
        DiffSource::Patch(PatchInput::File(path)) => {
            let text = fs::read_to_string(path)
                .with_context(|| format!("could not read patch file {}", path.display()))?;
            Ok(from_text(text))
        }
        DiffSource::Patch(PatchInput::Stdin) => {
            let mut text = String::new();
            io::stdin()
                .read_to_string(&mut text)
                .context("could not read patch from stdin")?;
            Ok(from_text(text))
        }
    }
}

fn from_text(text: String) -> LoadedDiff {
    let text = std::sync::Arc::<str>::from(text);
    LoadedDiff {
        document: std::sync::Arc::new(DiffDocument::parse(&text)),
        text,
        provenance: DiffProvenance::None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::load;
    use crate::domain::diff::{DiffRequest, DiffSource, PatchInput};

    #[test]
    fn patch_file_uses_the_common_loaded_diff_representation() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("revia-patch-{nonce}.patch"));
        let patch =
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n+new\n";
        fs::write(&path, patch).unwrap();
        let request = DiffRequest {
            source: DiffSource::Patch(PatchInput::File(path.clone())),
            context_lines: 3,
        };

        let loaded = load(Path::new("."), &request).unwrap();

        assert_eq!(loaded.text.as_ref(), patch);
        assert_eq!(loaded.document.files[0].path, "a.txt");
        fs::remove_file(path).unwrap();
    }
}
