use serde::Deserialize;

// derive Deserialize for the struct
#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub nsec: String,
    pub node_grpc_host: String,
    pub node_macaroon_path: String,
    pub node_tls_path: String,
    pub relays: Vec<String>,
    pub amount_sats: u64,
}

impl Config {
    pub fn new() -> Self {
        let config_path = std::env::args()
            .nth(1)
            .unwrap_or_else(|| String::from("config.yml"));

        match std::fs::File::open(config_path) {
            Ok(config_file) => {
                let config: Config =
                    serde_yaml::from_reader(config_file).expect("yaml formating error");
                config
            }
            Err(_) => {
                panic!("no config file found maybe read the readme");
            }
        }
    }
}
