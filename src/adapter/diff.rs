use std::{
    fs,
    io::{self, Read},
    path::Path,
};

use anyhow::{Context, Result};

use crate::domain::diff::{CapturedInput, DiffRequest, DiffSource, PatchContent, PatchInput};

use super::git::diff as git_diff;

pub fn capture(repository: &Path, request: &DiffRequest) -> Result<CapturedInput> {
    match &request.source {
        DiffSource::Git(comparison) => {
            git_diff::capture(repository, comparison, request.context_lines)
        }
        DiffSource::Patch(PatchInput::File(path)) => {
            let text = fs::read_to_string(path)
                .with_context(|| format!("could not read patch file {}", path.display()))?;
            Ok(CapturedInput::PatchFile {
                path: path.clone(),
                patch: PatchContent::new(text),
            })
        }
        DiffSource::Patch(PatchInput::Stdin) => {
            let mut text = String::new();
            io::stdin()
                .read_to_string(&mut text)
                .context("could not read patch from stdin")?;
            Ok(CapturedInput::PatchStdin {
                patch: PatchContent::new(text),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::capture;
    use crate::domain::diff::{DiffRequest, DiffSource, PatchInput};

    #[test]
    fn patch_file_capture_retains_its_source_and_exact_text() {
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

        let captured = capture(Path::new("."), &request).unwrap();

        assert_eq!(captured.patch().text(), patch);
        fs::remove_file(path).unwrap();
    }
}
