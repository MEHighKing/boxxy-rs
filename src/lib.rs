//! boxxy draws one persistent frame and runs the user's shell inside it.
//!
//! The shell is a normal process on a PTY. Its output is parsed into a screen
//! and painted only in the interior, so full-screen programs cannot overwrite
//! the border.

#![deny(unsafe_code)]

pub mod cli;
pub mod config;
pub mod frame;
pub mod keys;
pub mod session;

use std::io::{self, IsTerminal};

/// Run boxxy from the current process arguments. Returns the process exit code.
pub fn entry() -> i32 {
    match cli::parse(std::env::args_os()) {
        Err(err) => {
            eprintln!("{err}");
            2
        }
        Ok(cli::Command::Help) => {
            print!("{}", cli::HELP);
            0
        }
        Ok(cli::Command::Version) => {
            println!("boxxy {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Ok(cli::Command::Generate { force }) => match config::generate(force) {
            Ok(path) => {
                println!("Wrote {}", path.display());
                0
            }
            Err(err) => {
                eprintln!("{err}");
                1
            }
        },
        Ok(cli::Command::Run) => match config::load() {
            Err(err) => {
                eprintln!("{err}");
                1
            }
            Ok(config) => {
                if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                    eprintln!("Error:\nboxxy must be run in a terminal.");
                    return 1;
                }
                match session::run(&config) {
                    Ok(code) => code,
                    Err(err) => {
                        eprintln!("{err}");
                        1
                    }
                }
            }
        },
    }
}
