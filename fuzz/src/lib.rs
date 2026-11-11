//! Exercise the real file import APIs in a fresh directory; never use CAD caches.
pub fn with_input(data: &[u8], filename: &str, run: impl FnOnce(&std::path::Path)) {
    let dir = tempfile::tempdir().expect("create fuzz input directory");
    let path = dir.path().join(filename);
    std::fs::write(&path, data).expect("write fuzz input");
    run(&path);
}
