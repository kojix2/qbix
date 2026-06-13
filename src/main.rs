use std::process;

fn main() {
    if let Err(err) = qbix::cli::run(std::env::args()) {
        eprintln!("{err}");
        process::exit(1);
    }
}
