//! Console build of the command line, installed next to the app on Windows
//! (`cli-exe` feature). The statusline hook runs this one: PowerShell, which
//! Claude Code uses on Windows without Git Bash, does not wait for the GUI app.

fn main() {
    std::process::exit(limitlane_cli::run(std::env::args().skip(1).collect()));
}
