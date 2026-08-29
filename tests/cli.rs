use std::process::Command;

#[test]
fn version_prints_without_starting_the_app() {
    let output = Command::new(env!("CARGO_BIN_EXE_game-transcribe"))
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("game-transcribe {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}
