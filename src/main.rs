mod audio;
mod cli;
mod config;

fn main() {
    if let Err(error) = cli::run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}
