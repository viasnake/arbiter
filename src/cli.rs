pub(crate) enum Command {
    Serve {
        config_path: String,
    },
    AuditVerify {
        audit_path: String,
        mirror_path: Option<String>,
    },
    Invalid,
}

pub(crate) fn parse_args<I>(args: I) -> Command
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let cmd = args.next().unwrap_or_default();

    if cmd == "audit-verify" {
        return parse_audit_verify(args);
    }

    if cmd == "serve" {
        return parse_serve(args);
    }

    Command::Invalid
}

fn parse_audit_verify(mut args: impl Iterator<Item = String>) -> Command {
    let mut audit_path = String::from("./arbiter-audit.jsonl");
    let mut mirror_path: Option<String> = None;

    while let Some(arg) = args.next() {
        if arg == "--path" {
            if let Some(v) = args.next() {
                audit_path = v;
            }
        }
        if arg == "--mirror-path" {
            if let Some(v) = args.next() {
                mirror_path = Some(v);
            }
        }
    }

    Command::AuditVerify {
        audit_path,
        mirror_path,
    }
}

fn parse_serve(mut args: impl Iterator<Item = String>) -> Command {
    let mut config_path = String::from("./config/example-config.yaml");
    while let Some(arg) = args.next() {
        if arg == "--config" {
            if let Some(v) = args.next() {
                config_path = v;
            }
        }
    }
    Command::Serve { config_path }
}

#[cfg(test)]
mod tests {
    use super::{parse_args, Command};

    #[test]
    fn parse_serve_with_default_config() {
        match parse_args(vec!["serve".to_string()]) {
            Command::Serve { config_path } => {
                assert_eq!(config_path, "./config/example-config.yaml");
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_audit_verify_with_paths() {
        match parse_args(vec![
            "audit-verify".to_string(),
            "--path".to_string(),
            "./a.jsonl".to_string(),
            "--mirror-path".to_string(),
            "./m.jsonl".to_string(),
        ]) {
            Command::AuditVerify {
                audit_path,
                mirror_path,
            } => {
                assert_eq!(audit_path, "./a.jsonl");
                assert_eq!(mirror_path, Some("./m.jsonl".to_string()));
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_audit_verify_without_path_uses_default() {
        match parse_args(vec!["audit-verify".to_string()]) {
            Command::AuditVerify {
                audit_path,
                mirror_path,
            } => {
                assert_eq!(audit_path, "./arbiter-audit.jsonl");
                assert_eq!(mirror_path, None);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_invalid_command() {
        match parse_args(vec!["unknown".to_string()]) {
            Command::Invalid => {}
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn parse_serve_missing_config_value_keeps_default() {
        match parse_args(vec!["serve".to_string(), "--config".to_string()]) {
            Command::Serve { config_path } => {
                assert_eq!(config_path, "./config/example-config.yaml");
            }
            _ => panic!("unexpected command"),
        }
    }
}
