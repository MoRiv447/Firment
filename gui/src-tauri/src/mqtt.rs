//! MQTT data-plane link to the SBC broker (docs/sbc-agent.md §3,
//! gui-workbench.md §5). Subscribes `firment/#` and forwards device frames
//! and guard status to the frontend as global FrontendEvents.
//!
//! Deviation from the design note: this is a dedicated task emitting
//! FrontendEvents directly rather than a CollabBackend impl — the trait's
//! std-mpsc shape fits the multi-user review flow, not a live telemetry
//! feed, and the GUI needs zero coupling to it.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rumqttc::AsyncClient;

use crate::events::FrontendEvent;
use crate::state::Shared;

/// File-based link trace, independent of the frontend:
/// `%APPDATA%\firment\mqtt-link.log` (next to config.toml). Ends every
/// "is it even running" debate.
fn trace(shared: &Arc<Shared>, line: &str) {
    use std::io::Write as _;
    let path = firment_core::config::config_dir().join("mqtt-link.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{} {line}", Utc::now().to_rfc3339());
    }
    let _ = shared;
}

/// Spawn the MQTT link when `[mqtt] broker` is configured; announce loudly
/// otherwise, so a silent card can always be told apart from an unconfigured
/// one. Runs on its own thread + private single-thread runtime — immune to
/// anything weird in the host async runtime.
pub fn spawn_if_configured(shared: Arc<Shared>) {
    let broker = {
        let cfg = shared.config.lock().unwrap();
        cfg.mqtt.broker.trim().to_string()
    };
    trace(&shared, &format!("spawn: broker read as {broker:?}"));
    set_status(
        &shared,
        "{\"connected\":false,\"error\":\"link starting\"}".to_string(),
    );
    if broker.is_empty() {
        use tauri::Emitter as _;
        let _ = shared.app.emit(
            "agent-event",
            FrontendEvent::Info {
                session_id: None,
                message: "[mqtt] no [mqtt] broker configured — data-plane link off".to_string(),
            },
        );
        return;
    }
    let app = shared.app.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("mqtt runtime");
        rt.block_on(run(shared, broker));
        drop(app);
    });
}

/// Split "host:port" (port optional, default 1883). IPv6 literals are not
/// supported on purpose — the SBC sits on a LAN with a v4 address.
fn parse_broker(broker: &str) -> (String, u16) {
    match broker.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(1883)),
        None => (broker.to_string(), 1883),
    }
}

async fn run(shared: Arc<Shared>, broker: String) {
    let (host, port) = parse_broker(&broker);
    trace(&shared, &format!("link starting -> {host}:{port}"));
    emit(
        &shared,
        FrontendEvent::Info {
            session_id: None,
            message: format!("[mqtt] link starting -> {host}:{port}"),
        },
    );
    // Link state persists ACROSS reconnects: frames stays cumulative,
    // error_announced stays latched until a CONNACK clears it.
    let mut frames: u64 = 0;
    let mut error_announced = false;
    let mut connacked = false;
    let mut last_status = std::time::Instant::now() - Duration::from_secs(60);
    loop {
        // Client ID must be unique per process: two GUI instances with the
        // same id kick each other off the broker in an endless
        // connect/disconnect war (the "flickering status" bug).
        // Connection timeout defaults to 5s (NetworkOptions) — failures
        // surface quickly.
        let client_id = format!("firment-gui-{}", std::process::id());
        let opts = rumqttc::MqttOptions::new(&client_id, &host, port);
        let (client, mut eventloop) = {
            let (c, el) = AsyncClient::new(opts, 64);
            (c, el)
        };
        if let Err(e) = client
            .subscribe("firment/#", rumqttc::QoS::AtMostOnce)
            .await
        {
            trace(&shared, &format!("subscribe failed: {e}"));
            emit(
                &shared,
                FrontendEvent::Info {
                    session_id: None,
                    message: format!("[mqtt] subscribe failed: {e}"),
                },
            );
        } else {
            trace(&shared, "subscribe ok (queued; online on CONNACK)");
        }

        loop {
            match eventloop.poll().await {
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::Publish(p))) => {
                    frames += 1;
                    if frames == 1 {
                        trace(&shared, "first frame received");
                    }
                    forward(&shared, &p.topic, &p.payload);
                }
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))) => {
                    connacked = true;
                    error_announced = false; // recovered — next outage announces again
                    trace(&shared, "CONNACK — broker session established");
                    // Online ONLY after the broker actually acknowledged the
                    // session — "subscribe queued" used to flip the card to
                    // green before the handshake, then errors flipped it
                    // back: the flickering-status bug.
                    set_status(
                        &shared,
                        format!("{{\"connected\":true,\"broker\":\"{broker}\"}}"),
                    );
                    emit(
                        &shared,
                        FrontendEvent::GuardStatus {
                            frame: format!("{{\"connected\":true,\"broker\":\"{broker}\"}}"),
                        },
                    );
                }
                Ok(_) => {
                    if connacked && last_status.elapsed() >= Duration::from_secs(10) {
                        last_status = std::time::Instant::now();
                        trace(&shared, &format!("status heartbeat (frames={frames})"));
                        set_status(
                            &shared,
                            format!("{{\"connected\":true,\"broker\":\"{broker}\",\"frames\":{frames}}}"),
                        );
                        emit(&shared, FrontendEvent::GuardStatus {
                            frame: format!("{{\"connected\":true,\"broker\":\"{broker}\",\"frames\":{frames}}}"),
                        });
                    }
                }
                Err(e) => {
                    trace(
                        &shared,
                        &format!("eventloop error (connacked={connacked}, frames={frames}): {e}"),
                    );
                    set_status(
                        &shared,
                        format!("{{\"connected\":false,\"error\":\"{e}\"}}"),
                    );
                    emit(
                        &shared,
                        FrontendEvent::GuardStatus {
                            frame: format!("{{\"connected\":false,\"error\":\"{e}\"}}"),
                        },
                    );
                    // The Info line fires ONCE per outage (edge-triggered):
                    // with the SBC off, the 3s retry loop would otherwise
                    // spam the chat with the same error every cycle.
                    if !error_announced {
                        error_announced = true;
                        emit(
                            &shared,
                            FrontendEvent::Info {
                                session_id: None,
                                message: format!(
                                    "[mqtt] link error: {e} — retrying in 3s (will stop \
                                     announcing until it recovers)"
                                ),
                            },
                        );
                    }
                    // Backoff then rebuild the whole client: rumqttc event
                    // loops do not recover cleanly from transport errors.
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    break;
                }
            }
        }
    }
}

/// Topic routing: `firment/device/<node>/<kind>` and `firment/guard/status`.
/// Everything else (collab/presence, foreign namespaces) routes nowhere.
enum Route {
    Device { node: String, kind: String },
    GuardStatus,
}

fn route_topic(topic: &str) -> Option<Route> {
    let mut parts = topic.split('/');
    match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some("firment"), Some("device"), Some(node), Some(kind), None) => Some(Route::Device {
            node: node.to_string(),
            kind: kind.to_string(),
        }),
        (Some("firment"), Some("guard"), Some("status"), None, None) => Some(Route::GuardStatus),
        _ => None,
    }
}

fn forward(shared: &Arc<Shared>, topic: &str, payload: &[u8]) {
    let frame = String::from_utf8_lossy(payload).into_owned();
    match route_topic(topic) {
        Some(Route::Device { node, kind }) => {
            // Side-sink for the device_log tool: every frame lands in a
            // daily JSONL next to config.toml (single writer = this task,
            // so plain appends are safe).
            sink_frame(&frame);
            emit(shared, FrontendEvent::DeviceFrame { node, kind, frame });
        }
        Some(Route::GuardStatus) => {
            sink_frame(&frame);
            emit(shared, FrontendEvent::GuardStatus { frame });
        }
        None => {} // collab/presence etc. land in M4
    }
}

/// Append one raw frame to `device-log-<YYYYMMDD>.jsonl` in the config dir.
/// Best-effort: a logging failure never breaks the live feed. Single
/// writer (this MQTT task) so plain appends are safe.
fn sink_frame(frame: &str) {
    use std::io::Write as _;
    let day = chrono::Utc::now().format("%Y%m%d");
    let path = firment_core::config::config_dir().join(format!("device-log-{day}.jsonl"));
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let clean = frame.replace('\n', " ");
        let _ = writeln!(f, "{clean}");
    }
}

fn emit(shared: &Arc<Shared>, ev: FrontendEvent) {
    use tauri::Emitter as _;
    let _ = shared.app.emit("agent-event", ev);
}

/// Persist the last link status so a freshly attached frontend (or a card
/// remount) can pull the truth via the mqtt_status command instead of
/// waiting for the next event.
fn set_status(shared: &Arc<Shared>, frame: String) {
    *shared.mqtt_status.lock().unwrap() = frame;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_device_and_guard_topics() {
        match route_topic("firment/device/s3-node-1/telemetry") {
            Some(Route::Device { node, kind }) => {
                assert_eq!(node, "s3-node-1");
                assert_eq!(kind, "telemetry");
            }
            _ => panic!("device topic must route"),
        }
        assert!(matches!(
            route_topic("firment/guard/status"),
            Some(Route::GuardStatus)
        ));
        // Regression: the namespace segment is "firment" — the old matcher
        // compared the WRONG segment and silently dropped every frame.
        assert!(matches!(
            route_topic("firment/device/x/state"),
            Some(Route::Device { .. })
        ));
        assert!(route_topic("firment/collab/presence").is_none());
        assert!(route_topic("other/device/x/telemetry").is_none());
        assert!(route_topic("firment/device/x/telemetry/extra").is_none());
    }
}
