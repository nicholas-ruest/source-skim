use serde_json::Value;
use std::fs;
use std::process::Command;

#[test]
fn compile_verify_and_render_vertical_slice() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("manifest.json");
    let rendered = temp.path().join("context.txt");
    let example = format!("{}/examples/demo-bundle.json", env!("CARGO_MANIFEST_DIR"));
    let binary = env!("CARGO_BIN_EXE_source-skim");

    let status = Command::new(binary)
        .args([
            "compile",
            "--input",
            &example,
            "--budget",
            "22",
            "--min-sources",
            "2",
            "--output",
        ])
        .arg(&manifest)
        .status()
        .unwrap();
    assert!(status.success());

    let parsed: Value = serde_json::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    assert_eq!(parsed["total_tokens"], 22);
    assert_eq!(parsed["sources_covered"], 2);

    let verify = Command::new(binary)
        .args(["verify", "--input", &example, "--manifest"])
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(verify.status.success());
    assert!(String::from_utf8(verify.stdout)
        .unwrap()
        .contains("verified 2 spans, 22 tokens, 2 sources"));

    let status = Command::new(binary)
        .args(["render", "--input", &example, "--manifest"])
        .arg(&manifest)
        .arg("--output")
        .arg(&rendered)
        .status()
        .unwrap();
    assert!(status.success());
    let context = fs::read_to_string(rendered).unwrap();
    assert!(context.contains("source:relay-notes"));
    assert!(context.contains("source:research"));
    assert!(context
        .contains("A successful completion marker was never emitted after a partial failure."));
    assert!(context.contains("Every retained span still needs source-addressable provenance."));
}
