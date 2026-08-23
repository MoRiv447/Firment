// ESP32-C3/S3 SuperMini — MQTT telemetry node (P0-4 of docs/sbc-agent.md §5)
//
// Publishes a heartbeat + a fake sensor metric to the SBC broker every 5 s,
// subscribes to its command topic and echoes payloads back as telemetry.
//
// Build (Arduino IDE or arduino-cli):
//   board: "ESP32C3 Dev Module"   (S3: "ESP32S3 Dev Module")
//   libs : PubSubClient (Nick O'Leary)
//
// Flash from the SBC itself (after arduino-cli is installed there) or from
// any desktop with the board plugged in:
//   arduino-cli upload -p <PORT> --input-dir . --fqbn esp32:esp32:esp32c3 main.ino

#include <WiFi.h>
#include <PubSubClient.h>

// ---- fill in per deployment (or use your own wifi-manager) --------------
const char* WIFI_SSID = "YOUR_WIFI_SSID";
const char* WIFI_PASS = "YOUR_WIFI_PASS";
// SBC broker (see docs/sbc-agent.md §3.1)
const char* MQTT_HOST = "192.168.1.6";
const uint16_t MQTT_PORT = 1883;
const char* NODE_NAME = "c3-node-1";
// -------------------------------------------------------------------------

WiFiClient wifi;
PubSubClient mqtt(wifi);

unsigned long last_pub = 0;
uint32_t seq = 0;

void publish(const char* kind, const char* payload) {
  char topic[64];
  snprintf(topic, sizeof(topic), "firment/device/%s/%s", NODE_NAME, kind);
  char msg[192];
  snprintf(msg, sizeof(msg),
           "{\"node\":\"%s\",\"kind\":\"%s\",\"seq\":%u,\"payload\":\"%s\"}",
           NODE_NAME, kind, seq++, payload);
  mqtt.publish(topic, msg);
}

void on_message(char* topic, byte* body, unsigned int len) {
  char cmd[128];
  unsigned int n = len < sizeof(cmd) - 1 ? len : sizeof(cmd) - 1;
  memcpy(cmd, body, n);
  cmd[n] = 0;
  publish("echo", cmd);  // prove downlink works
}

void connect_wifi() {
  WiFi.mode(WIFI_STA);
  Serial.printf("[wifi] connecting to %s", WIFI_SSID);
  WiFi.begin(WIFI_SSID, WIFI_PASS);
  int waited = 0;
  while (WiFi.status() != WL_CONNECTED) {
    delay(300);
    waited += 300;
    Serial.print(WiFi.status());
    if (waited > 15000) {
      Serial.println();
      Serial.printf("[wifi] FAILED status=%d — check SSID/band (ESP32 is 2.4GHz only)\n",
                    WiFi.status());
      waited = 0;  // keep retrying so a later fix lands without re-flash
    }
  }
  Serial.printf("\n[wifi] connected, IP: %s\n", WiFi.localIP().toString().c_str());
}

void connect_mqtt() {
  while (!mqtt.connected()) {
    Serial.printf("[mqtt] connecting to %s:%u ...\n", MQTT_HOST, MQTT_PORT);
    if (mqtt.connect(NODE_NAME)) {
      char topic[64];
      snprintf(topic, sizeof(topic), "firment/device/%s/cmd", NODE_NAME);
      mqtt.subscribe(topic);
      Serial.println("[mqtt] connected");
    } else {
      Serial.printf("[mqtt] failed rc=%d, retry in 1.5s\n", mqtt.state());
      delay(1500);
    }
  }
}

void setup() {
  Serial.begin(115200);
  delay(200);
  Serial.println("\n[node] boot");
  connect_wifi();
  mqtt.setServer(MQTT_HOST, MQTT_PORT);
  mqtt.setCallback(on_message);
  connect_mqtt();
  Serial.println("[node] ready");
}

void loop() {
  if (!mqtt.connected()) connect_mqtt();
  mqtt.loop();
  unsigned long now = millis();
  if (now - last_pub >= 5000) {
    last_pub = now;
    // Stand-in for a real sensor: internal temp in mV-ish raw reading.
    publish("telemetry", "raw=1234");
  }
}
