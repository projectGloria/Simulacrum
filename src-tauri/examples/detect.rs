//! Print what the Directory reader finds in each folder given on the command line (read-only).
//!
//! `cargo run --example detect -- <dir> [<dir> …]`

fn main() {
    for dir in std::env::args().skip(1) {
        match simulacrum_lib::project::scan(std::path::Path::new(&dir)) {
            Ok(r) => {
                println!("{}  →  {}", r.name, r.primary_label.as_deref().unwrap_or("(nothing)"));
                for d in &r.detections {
                    println!("  [{}] evidence: {}", d.label, d.evidence.join(", "));
                    for t in &d.targets {
                        let tag = if t.exists { "replace" } else { "new" };
                        let on = if t.default_on { "x" } else { " " };
                        println!("    [{on}] {:<7} {}  {:?} {:?}", tag, t.rel_path, t.format, t.profile);
                    }
                    for n in &d.notes {
                        println!("    note: {n}");
                    }
                }
                for w in &r.warnings {
                    println!("  warning: {w}");
                }
            }
            Err(e) => println!("{dir}: error: {e}"),
        }
    }
}
