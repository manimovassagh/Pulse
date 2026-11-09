//! Compiles the counter example to Rust, builds the generated crate for the
//! host target (where `pulse-runtime` uses its in-memory DOM) and runs a test
//! inside it that renders the app and clicks buttons.
//!
//! This checks the generated code end to end without a browser; the browser
//! test lives in `tests/e2e/counter.test.mjs`.

use pulse_compiler::driver;
use std::path::Path;
use std::process::Command;

const HARNESS: &str = r##"
#[cfg(test)]
mod native_harness {
    use super::*;
    use pulse_runtime::dom::mock;

    fn html() -> String {
        mock::mounted_html("#app").expect("app mounted")
    }

    fn button(id: &str) -> Node {
        let root = mock::root("#app").unwrap();
        mock::find_all(root, "button")
            .into_iter()
            .find(|b| mock::html(*b).starts_with(&format!("<button id=\"{id}\"")))
            .unwrap_or_else(|| panic!("button #{id} not found in {}", html()))
    }

    #[test]
    fn counter_renders_and_reacts() {
        pulse_start();
        let h = html();
        assert!(h.contains("<h1>Pulse Counter</h1>"), "{h}");
        assert!(h.contains(">Count: 0</button>"), "{h}");
        assert!(h.contains("<button id=\"reset\" disabled=\"\">"), "{h}");

        mock::fire(button("increment"), "click");
        let h = html();
        assert!(h.contains(">Count: 1</button>"), "{h}");
        assert!(h.contains("You clicked 1 click."), "{h}");
        assert!(h.contains("Doubled: 2 · Parity: odd"), "{h}");
        assert!(h.contains("<button id=\"reset\">"), "{h}");

        mock::fire(button("step"), "click");
        mock::fire(button("increment"), "click");
        assert!(html().contains(">Count: 3</button>"));

        mock::fire(button("reset"), "click");
        assert!(html().contains(">Count: 0</button>"));
        assert_eq!(
            mock::console_messages(),
            ["count changed to 0", "count changed to 1", "count changed to 3", "count changed to 0"]
        );
    }
}
"##;

#[test]
fn generated_counter_app_runs_natively() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let entry = root.join("examples/counter/src/App.tsx");
    let source = std::fs::read_to_string(&entry).unwrap();
    let rust = driver::compile_to_rust(&source, "App.tsx").unwrap_or_else(|e| panic!("{e}"));

    let work = Path::new(env!("CARGO_TARGET_TMPDIR")).join("native_counter");
    driver::write_app_crate(&work, &format!("{rust}\n{HARNESS}"), &driver::default_runtime_path()).unwrap();

    let output = Command::new(env!("CARGO"))
        .args(["test", "--quiet", "--manifest-path"])
        .arg(work.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(work.join("target"))
        .output()
        .expect("run cargo");
    assert!(
        output.status.success(),
        "generated crate failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
