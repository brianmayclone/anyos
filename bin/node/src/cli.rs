use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, Default)]
pub struct NodeCli {
    pub mode: NodeMode,
    pub exec_argv: Vec<String>,
    pub argv_tail: Vec<String>,
    pub preloads: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum NodeMode {
    #[default]
    Repl,
    Help,
    Version,
    Eval { source: String, print: bool },
    Check { script: String },
    Script { script: String },
    Stdin,
}

pub fn parse(raw: &str) -> Result<NodeCli, String> {
    let tokens = anyos_std::args::tokenize(raw);
    let mut cli = NodeCli::default();
    if tokens.is_empty() {
        return Ok(cli);
    }

    let mut idx = 0usize;
    let mut stop_options = false;
    while idx < tokens.len() {
        let token = &tokens[idx];
        if stop_options {
            return script_from(idx, &tokens, cli);
        }
        if token == "--" {
            stop_options = true;
            idx += 1;
            continue;
        }
        if token == "-" {
            cli.mode = NodeMode::Stdin;
            cli.argv_tail = tokens[idx + 1..].to_vec();
            return Ok(cli);
        }
        if !token.starts_with('-') || token == "-" {
            return script_from(idx, &tokens, cli);
        }

        match token.as_str() {
            "-v" | "--version" => {
                cli.mode = NodeMode::Version;
                return Ok(cli);
            }
            "-h" | "--help" => {
                cli.mode = NodeMode::Help;
                return Ok(cli);
            }
            "-i" | "--interactive" => {
                cli.mode = NodeMode::Repl;
                cli.exec_argv.push(token.clone());
                idx += 1;
            }
            "-c" | "--check" => {
                let script = take_value(&tokens, &mut idx, token)?;
                cli.exec_argv.push(token.clone());
                cli.mode = NodeMode::Check { script };
                cli.argv_tail = tokens[idx + 1..].to_vec();
                return Ok(cli);
            }
            "-e" | "--eval" => {
                let source = take_value(&tokens, &mut idx, token)?;
                cli.exec_argv.push(token.clone());
                cli.exec_argv.push(source.clone());
                cli.mode = NodeMode::Eval {
                    source,
                    print: false,
                };
                cli.argv_tail = tokens[idx + 1..].to_vec();
                return Ok(cli);
            }
            "-p" | "--print" => {
                let source = take_value(&tokens, &mut idx, token)?;
                cli.exec_argv.push(token.clone());
                cli.exec_argv.push(source.clone());
                cli.mode = NodeMode::Eval {
                    source,
                    print: true,
                };
                cli.argv_tail = tokens[idx + 1..].to_vec();
                return Ok(cli);
            }
            "-r" | "--require" => {
                let module = take_value(&tokens, &mut idx, token)?;
                cli.exec_argv.push(token.clone());
                cli.exec_argv.push(module.clone());
                cli.preloads.push(module);
                idx += 1;
            }
            "--trace-warnings"
            | "--no-warnings"
            | "--enable-source-maps"
            | "--experimental-modules"
            | "--experimental-fetch"
            | "--no-deprecation"
            | "--pending-deprecation"
            | "--preserve-symlinks"
            | "--preserve-symlinks-main" => {
                cli.exec_argv.push(token.clone());
                idx += 1;
            }
            _ if token.starts_with("--eval=") => {
                let source = String::from(&token["--eval=".len()..]);
                cli.exec_argv.push(String::from("--eval"));
                cli.exec_argv.push(source.clone());
                cli.mode = NodeMode::Eval {
                    source,
                    print: false,
                };
                cli.argv_tail = tokens[idx + 1..].to_vec();
                return Ok(cli);
            }
            _ if token.starts_with("--print=") => {
                let source = String::from(&token["--print=".len()..]);
                cli.exec_argv.push(String::from("--print"));
                cli.exec_argv.push(source.clone());
                cli.mode = NodeMode::Eval {
                    source,
                    print: true,
                };
                cli.argv_tail = tokens[idx + 1..].to_vec();
                return Ok(cli);
            }
            _ if token.starts_with("--require=") => {
                let module = String::from(&token["--require=".len()..]);
                cli.exec_argv.push(String::from("--require"));
                cli.exec_argv.push(module.clone());
                cli.preloads.push(module);
                idx += 1;
            }
            _ if token.starts_with("-e") && token.len() > 2 => {
                let source = String::from(&token[2..]);
                cli.exec_argv.push(String::from("-e"));
                cli.exec_argv.push(source.clone());
                cli.mode = NodeMode::Eval {
                    source,
                    print: false,
                };
                cli.argv_tail = tokens[idx + 1..].to_vec();
                return Ok(cli);
            }
            _ if token.starts_with("-p") && token.len() > 2 => {
                let source = String::from(&token[2..]);
                cli.exec_argv.push(String::from("-p"));
                cli.exec_argv.push(source.clone());
                cli.mode = NodeMode::Eval {
                    source,
                    print: true,
                };
                cli.argv_tail = tokens[idx + 1..].to_vec();
                return Ok(cli);
            }
            _ => return Err(alloc::format!("bad option: {}", token)),
        }
    }
    Ok(cli)
}

fn script_from(idx: usize, tokens: &[String], mut cli: NodeCli) -> Result<NodeCli, String> {
    let Some(script) = tokens.get(idx).cloned() else {
        cli.mode = NodeMode::Repl;
        return Ok(cli);
    };
    cli.argv_tail = tokens[idx + 1..].to_vec();
    cli.mode = NodeMode::Script { script };
    Ok(cli)
}

fn take_value(tokens: &[String], idx: &mut usize, option: &str) -> Result<String, String> {
    let value_idx = *idx + 1;
    let Some(value) = tokens.get(value_idx) else {
        return Err(alloc::format!("{} requires an argument", option));
    };
    *idx = value_idx;
    Ok(value.clone())
}

