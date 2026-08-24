//! Standalone probe: print the exact config path + [mqtt] section the GUI
//! would read, so "silent link" cases can be diagnosed without launching
//! the app. Run: cargo run -p firment-core --example mqtt_probe

fn main() {
    let dir = firment_core::config::config_dir();
    println!("config dir : {}", dir.display());
    println!("  (override with FIRMENT_CONFIG_DIR env var)");
    println!(
        "FIRMENT_CONFIG_DIR env = {:?}",
        std::env::var("FIRMENT_CONFIG_DIR")
    );
    let path = firment_core::config::config_path();
    println!(
        "config file: {} (exists: {})",
        path.display(),
        path.exists()
    );
    match firment_core::Config::load_or_create(&path) {
        Ok(cfg) => {
            println!("parsed mqtt.broker = {:?}", cfg.mqtt.broker);
            if cfg.mqtt.broker.is_empty() {
                println!(">>> broker EMPTY: the mqtt link will not start");
            }
        }
        Err(e) => println!("CONFIG LOAD ERROR: {e}"),
    }
}
