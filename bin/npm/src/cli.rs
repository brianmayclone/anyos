use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NpmCommand {
    Help,
    Version,
    Init { yes: bool },
    Install { packages: Vec<String> },
    Uninstall { packages: Vec<String> },
    Update { packages: Vec<String> },
    Run { script: String, args: Vec<String> },
    List,
    Outdated,
    Info { package: String },
    Search { query: String },
}

#[derive(Clone, Debug)]
pub struct NpmCli {
    pub command: NpmCommand,
    pub registry: String,
    pub global: bool,
    pub prefix: Option<String>,
}

pub fn parse(raw: &str) -> Result<NpmCli, String> {
    let tokens = anyos_std::args::tokenize(raw);
    let mut registry = String::from(libnode::DEFAULT_NPM_REGISTRY);
    let mut global = false;
    let mut prefix = None;
    let mut command: Option<String> = None;
    let mut positional = Vec::new();
    let mut yes = false;
    let mut idx = 0usize;

    while idx < tokens.len() {
        let token = &tokens[idx];
        match token.as_str() {
            "-v" | "--version" => {
                return Ok(NpmCli {
                    command: NpmCommand::Version,
                    registry,
                    global,
                    prefix,
                });
            }
            "-h" | "--help" | "help" => {
                return Ok(NpmCli {
                    command: NpmCommand::Help,
                    registry,
                    global,
                    prefix,
                });
            }
            "-g" | "--global" => {
                global = true;
                idx += 1;
            }
            "-y" | "--yes" => {
                yes = true;
                idx += 1;
            }
            "-r" | "--registry" => {
                idx += 1;
                let Some(value) = tokens.get(idx) else {
                    return Err(String::from("--registry requires a value"));
                };
                registry = value.clone();
                idx += 1;
            }
            _ if token.starts_with("--registry=") => {
                registry = String::from(&token["--registry=".len()..]);
                idx += 1;
            }
            "--prefix" => {
                idx += 1;
                let Some(value) = tokens.get(idx) else {
                    return Err(String::from("--prefix requires a value"));
                };
                prefix = Some(value.clone());
                idx += 1;
            }
            _ if token.starts_with("--prefix=") => {
                prefix = Some(String::from(&token["--prefix=".len()..]));
                idx += 1;
            }
            _ if token == "--location=global" => {
                global = true;
                idx += 1;
            }
            _ if is_ignored_install_flag(token) => {
                idx += 1;
                if option_consumes_next(token) && idx < tokens.len() {
                    idx += 1;
                }
            }
            _ if token.starts_with('-') => {
                return Err(alloc::format!("unsupported npm option: {}", token));
            }
            _ => {
                if command.is_none() {
                    command = Some(token.clone());
                } else {
                    positional.push(token.clone());
                }
                idx += 1;
            }
        }
    }

    let command = match command.as_deref().unwrap_or("help") {
        "init" => NpmCommand::Init { yes },
        "install" | "i" | "add" => NpmCommand::Install {
            packages: positional,
        },
        "uninstall" | "remove" | "rm" | "r" | "un" => NpmCommand::Uninstall {
            packages: positional,
        },
        "update" | "up" | "upgrade" => NpmCommand::Update {
            packages: positional,
        },
        "run" | "run-script" => {
            let Some(script) = positional.first().cloned() else {
                return Err(String::from("npm run requires a script name"));
            };
            NpmCommand::Run {
                script,
                args: positional.iter().skip(1).cloned().collect(),
            }
        }
        "start" => NpmCommand::Run {
            script: String::from("start"),
            args: positional,
        },
        "test" | "t" => NpmCommand::Run {
            script: String::from("test"),
            args: positional,
        },
        "list" | "ls" | "ll" | "la" => NpmCommand::List,
        "outdated" => NpmCommand::Outdated,
        "info" | "view" | "show" => {
            let Some(package) = positional.first().cloned() else {
                return Err(String::from("npm info requires a package name"));
            };
            NpmCommand::Info { package }
        }
        "search" | "s" => NpmCommand::Search {
            query: positional.join(" "),
        },
        "version" => NpmCommand::Version,
        "help" => NpmCommand::Help,
        other => return Err(alloc::format!("unknown npm command: {}", other)),
    };
    Ok(NpmCli {
        command,
        registry,
        global,
        prefix,
    })
}

fn is_ignored_install_flag(token: &str) -> bool {
    matches!(
        token,
        "--save"
            | "--save-prod"
            | "--save-dev"
            | "-D"
            | "--save-optional"
            | "-O"
            | "--save-peer"
            | "-P"
            | "--save-exact"
            | "-E"
            | "--package-lock"
            | "--no-package-lock"
            | "--legacy-peer-deps"
            | "--force"
            | "--audit"
            | "--no-audit"
            | "--fund"
            | "--no-fund"
            | "--ignore-scripts"
            | "--foreground-scripts"
    ) || token.starts_with("--omit=")
        || token.starts_with("--include=")
        || token.starts_with("--save-prefix=")
}

fn option_consumes_next(token: &str) -> bool {
    matches!(token, "--omit" | "--include" | "--save-prefix")
}
