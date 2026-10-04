use std::{fs, path::PathBuf};

pub(crate) fn temp_dir(prefix: &str) -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-temp");
    fs::create_dir_all(&root).expect("could not create the test temporary root");
    tempfile::Builder::new()
        .prefix(&format!("{prefix}-"))
        .tempdir_in(root)
        .expect("could not create a unique test temporary directory")
}
