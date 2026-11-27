fn main() {
    std::process::exit(ctc_cli::run(std::env::args().skip(1).collect()));
}
