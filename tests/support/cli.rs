use std::{path::PathBuf, process::Command, sync::OnceLock};

/// Cargo only exposes CARGO_BIN_EXE_* to tests in the binary's own package.
/// Build the workspace CLI explicitly so facade tests also work on their own.
pub fn executable() -> &'static PathBuf {
    static CLI: OnceLock<PathBuf> = OnceLock::new();
    CLI.get_or_init(|| {
        let output = Command::new(env!("CARGO"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args([
                "build",
                "--locked",
                "-p",
                "raytrace-cli",
                "--bin",
                "raytrace-rs",
                "--message-format=json",
            ])
            .output()
            .expect("build workspace CLI");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .find_map(|artifact| {
                (artifact["reason"] == "compiler-artifact"
                    && artifact["target"]["name"] == "raytrace-rs")
                    .then(|| artifact["executable"].as_str().map(PathBuf::from))
                    .flatten()
            })
            .expect("Cargo reported the CLI executable")
    })
}
