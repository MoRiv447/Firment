//! Headless MQTT probe using the same rumqttc path as the GUI link.
//! Run: cargo run -p firment-gui --example mqtt_sub_probe [broker]
//! Prints every incoming frame for 12 seconds, then exits.

use std::time::Duration;

#[tokio::main]
async fn main() {
    let broker = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "192.168.1.6:1883".into());
    let (host, port) = match broker.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(1883)),
        None => (broker.clone(), 1883),
    };
    println!("[probe] connecting to {host}:{port} ...");
    let (client, mut eventloop) = rumqttc::AsyncClient::new(
        rumqttc::MqttOptions::new("firment-probe", &host, port),
        16,
    );
    if let Err(e) = client.subscribe("firment/#", rumqttc::QoS::AtMostOnce).await {
        println!("[probe] subscribe ERR: {e}");
        return;
    }
    println!("[probe] subscribed, listening 12s ...");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    let mut count = 0usize;
    loop {
        let next = tokio::time::sleep_until(deadline);
        tokio::select! {
            _ = next => break,
            ev = eventloop.poll() => match ev {
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p))) => {
                    count += 1;
                    println!(
                        "[frame {count}] {} {}",
                        p.topic,
                        String::from_utf8_lossy(&p.payload)
                    );
                }
                Ok(rumqttc::Event::Incoming(_)) => {}
                Ok(_) => {}
                Err(e) => {
                    println!("[probe] eventloop ERR: {e}");
                    break;
                }
            }
        }
    }
    println!("[probe] total frames: {count}");
}
