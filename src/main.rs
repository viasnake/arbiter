use std::env;

#[tokio::main]
async fn main() {
    let mut args = env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    if cmd != "serve" {
        eprintln!("Usage: arbiter serve --config <path>");
        std::process::exit(2);
    }

    let mut config_path = String::from("./config/example-config.yaml");
    while let Some(arg) = args.next() {
        if arg == "--config" {
            if let Some(v) = args.next() {
                config_path = v;
            }
        }
    }

    let cfg = match arbiter_config::load_and_validate(&config_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed to load config: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = arbiter_server::serve(cfg).await {
        eprintln!("server exited with error: {e}");
        std::process::exit(1);
    }
}
