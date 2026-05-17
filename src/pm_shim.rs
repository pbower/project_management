//! Deprecation shim for the legacy `pm` binary name. Prints a one-line
//! notice to stderr and execs `spacecell` with the same arguments so v0.9
//! callers see a clean transition. Removed in v0.3.0.

use std::env;
use std::process::Command;

fn main() {
    eprintln!(
        "pm: deprecated binary name. Use `spacecell` (or alias `sc`) instead. \
         This shim is removed in v0.3.0."
    );

    let args: Vec<String> = env::args().skip(1).collect();
    let status = Command::new("spacecell")
        .args(&args)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("pm: cannot exec `spacecell`: {e}. Is the `spacecell` binary on PATH?");
            std::process::exit(127);
        });

    std::process::exit(status.code().unwrap_or(1));
}
