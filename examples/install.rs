//! Bring a TUI folder up to date from a release's workspace by hand, as the process does each hour, and print what
//! it did: `cargo run --example install -- <workspace> <folder>`. For checking a release against a copy of a node's
//! folder before it ships.

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [seed, dest] = args.as_slice() else { anyhow::bail!("usage: install <workspace> <folder>") };
    let r = tuis::install(std::path::Path::new(seed), std::path::Path::new(dest))?;
    println!("wrote: {}\nremoved: {}\nproblems: {}", r.written.join(", "), r.removed.join(", "), r.problems.join("; "));
    println!("apps: {}", tuis::tuis(std::path::Path::new(dest)).join(", "));
    Ok(())
}
