use colored::Colorize;

const BANNER: &str = r"
       _ _ _
      (_) | |
  __ _ _| | | ___  _   _
 / _` | | | |/ _ \| | | |
| (_| | | | | (_) | |_| |
 \__,_|_|_|_|\___/ \__, |
                     __/ |
                    |___/";

pub fn print_banner() {
    println!("{}", BANNER.trim_start_matches('\n').cyan());
    println!(
        "  {} {}",
        "ailloy".bold(),
        env!("CARGO_PKG_VERSION").dimmed()
    );
    println!("  {}", "An AI abstraction layer for Rust".dimmed());
    println!();
}
