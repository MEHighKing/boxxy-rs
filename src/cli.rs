use std::ffi::OsString;

pub const HELP: &str = "\
boxxy - a branded frame around your shell

Usage:
    boxxy
    boxxy --generate-config [--force]
    boxxy --help
    boxxy --version

Options:
    --generate-config    Write ~/.config/boxxycli/boxxy.toml
    --force              Replace an existing configuration file
    -h, --help           Show this help
    -V, --version        Show the version
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run,
    Generate { force: bool },
    Help,
    Version,
}

#[derive(Debug)]
pub struct UsageError {
    message: String,
}

impl UsageError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Error:\n{}\n\nRun 'boxxy --help' for usage.",
            self.message
        )
    }
}

impl std::error::Error for UsageError {}

/// Parse arguments after the program name.
pub fn parse<I>(args: I) -> Result<Command, UsageError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut rest = args.into_iter();
    let _program = rest.next();

    let mut generate = false;
    let mut force = false;
    let mut help = false;
    let mut version = false;

    for arg in rest {
        let Some(arg) = arg.to_str() else {
            return Err(UsageError::new("arguments must be valid Unicode"));
        };
        match arg {
            "--generate-config" => {
                if generate {
                    return Err(UsageError::new("duplicate argument: --generate-config"));
                }
                generate = true;
            }
            "--force" => {
                if force {
                    return Err(UsageError::new("duplicate argument: --force"));
                }
                force = true;
            }
            "-h" | "--help" => help = true,
            "-V" | "--version" => version = true,
            other if other.starts_with('-') => {
                return Err(UsageError::new(format!("unknown argument: {other}")));
            }
            other => {
                return Err(UsageError::new(format!("unexpected argument: {other}")));
            }
        }
    }

    let modes = [help, version, generate]
        .into_iter()
        .filter(|on| *on)
        .count();
    if modes > 1 {
        return Err(UsageError::new(
            "--help, --version, and --generate-config are separate commands",
        ));
    }
    if force && !generate {
        return Err(UsageError::new("--force requires --generate-config"));
    }

    if help {
        Ok(Command::Help)
    } else if version {
        Ok(Command::Version)
    } else if generate {
        Ok(Command::Generate { force })
    } else {
        Ok(Command::Run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(args: &[&str]) -> Result<Command, UsageError> {
        let owned = std::iter::once(OsString::from("boxxy"))
            .chain(args.iter().map(|arg| OsString::from(*arg)));
        parse(owned)
    }

    #[test]
    fn defaults_to_run() {
        assert_eq!(parse_str(&[]).unwrap(), Command::Run);
    }

    #[test]
    fn generate_flags() {
        assert_eq!(
            parse_str(&["--generate-config"]).unwrap(),
            Command::Generate { force: false }
        );
        assert_eq!(
            parse_str(&["--force", "--generate-config"]).unwrap(),
            Command::Generate { force: true }
        );
    }

    #[test]
    fn rejects_force_alone_and_unknown_flags() {
        assert!(parse_str(&["--force"]).is_err());
        assert!(parse_str(&["--theme", "catppuccin"]).is_err());
        assert!(parse_str(&["shell"]).is_err());
    }
}
