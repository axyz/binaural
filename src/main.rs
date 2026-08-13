mod audio;
mod cli;
mod config;
mod daemon;
mod ipc;
mod playback;

fn main() {
    if let Err(error) = cli::run() {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}
