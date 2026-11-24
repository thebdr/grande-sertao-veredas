//! Oficina's executable: the window, with the process's arguments (the first that is not an option
//! names the project folder to open). Built with the `shell` feature.

fn main() {
    std::process::exit(veredas_oficina::run());
}
