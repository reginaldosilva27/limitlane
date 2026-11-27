fn main() {
    std::process::exit(limitlane_cli::run(std::env::args().skip(1).collect()));
}
