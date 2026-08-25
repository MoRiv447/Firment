// ESP32-C3/S3 SuperMini — MQTT telemetry + command node
// (docs/sbc-agent.md §3; command protocol v1 = JSON envelope)
//
// Telemetry: heartbeat + fake sensor metric every 5 s on
//            firment/device/<node>/telemetry.
// Commands : JSON envelopes on firment/device/<node>/cmd —
//            {"cmd":"ping"}
//            {"cmd":"rgb.on"}  /  {"cmd":"rgb.off"}
//            {"cmd":"rgb.set","args":{"hex":"ff0000"}}
//            EVERY command is acknowledged on .../<node>/state (retained):
//            {"kind":"state","ack":{"cmd":...,"ok":true|false},"rgb":{...}}
//            Unknown/invalid commands ack with ok:false — never silent.
// Caps     : on boot publishes RETAINED .../<node>/caps advertising the
//            supported commands so agents can discover the grammar:
//            {"kind":"caps","cmds":[...],"args":{"rgb.set":["hex"]}}
// Legacy   : plain-text rgb:on|off|#RRGGBB still accepted (deprecated).
//
// Build (Arduino IDE or arduino-cli):
//   board: "ESP32C3 Dev Module"   (S3: "ESP32S3 Dev Module")
//   libs : PubSubClient (Nick O'Leary), ArduinoJson (Benoit Blanchon)
//   note : neopixelWrite() is builtin to arduino-esp32 >= 2.0.9
//
// Flash from the SBC itself (after arduino-cli is installed there) or from
// any desktop with the board plugged in:
//   arduino-cli compile --fqbn "esp32:esp32:esp32s3:CDCOnBoot=cdc" docs\examples\esp32c3-mqtt-node
//   arduino-cli upload  -p <PORT> --fqbn "esp32:esp32:esp32s3:CDCOnBoot=cdc" docs\examples\esp32c3-mqtt-node

#include <WiFi.h>
#include <PubSubClient.h>
#include <ArduinoJson.h>

// ---- fill in per deployment (or use your own wifi-manager) --------------
const char* WIFI_SSID = "CMCC-666";
const char* WIFI_PASS = "66666666";
// SBC broker (see docs/sbc-agent.md §3.1)
const char* MQTT_HOST = "192.168.1.6";
const uint16_t MQTT_PORT = 1883;
const char* NODE_NAME = "s3-node-1";
// -------------------------------------------------------------------------

#if defined(CONFIG_IDF_TARGET_ESP32S3)
#define RGB_PIN 48  // WS2812 on S3 SuperMini boards
#else
#define RGB_PIN 8   // typical C3 SuperMini WS2812
#endif

WiFiClient wifi;
PubSubClient mqtt(wifi);

unsigned long last_pub = 0;
uint32_t seq = 0;

// ---- rgb state ----------------------------------------------------------
bool rgb_on = false;
uint8_t rgb_r = 255, rgb_g = 255, rgb_b = 255;
char rgb_hex[8] = "#ffffff";

void apply_rgb() {
  if (rgb_on) {
    neopixelWrite(RGB_PIN, rgb_r, rgb_g, rgb_b);
  } else {
    neopixelWrite(RGB_PIN, 0, 0, 0);
  }
}

void rgb_hex_to_parts(const char* hex) {
  if (hex[0] == '#') hex++;
  auto byte_at = [&](int i) -> uint8_t {
    char buf[3] = {hex[i], hex[i + 1], 0};
    return (uint8_t)strtol(buf, nullptr, 16);
  };
  rgb_r = byte_at(0);
  rgb_g = byte_at(2);
  rgb_b = byte_at(4);
  snprintf(rgb_hex, sizeof(rgb_hex), "#%02x%02x%02x", rgb_r, rgb_g, rgb_b);
}

void publish_state(const char* ack_cmd = nullptr, bool ack_ok = true,
                   const char* ack_err = nullptr) {
  char topic[64];
  snprintf(topic, sizeof(topic), "firment/device/%s/state", NODE_NAME);
  // ack is optional: command responses carry it, plain state publishes
  // don't. Retained either way — late joiners see current state + last ack.
  char msg[256];
  if (ack_cmd) {
    if (ack_ok) {
      snprintf(msg, sizeof(msg),
               "{\"node\":\"%s\",\"kind\":\"state\",\"ack\":{\"cmd\":\"%s\",\"ok\":true},"
               "\"rgb\":{\"state\":\"%s\",\"hex\":\"%s\",\"r\":%u,\"g\":%u,\"b\":%u}}",
               NODE_NAME, ack_cmd, rgb_on ? "on" : "off", rgb_hex, rgb_r, rgb_g, rgb_b);
    } else {
      snprintf(msg, sizeof(msg),
               "{\"node\":\"%s\",\"kind\":\"state\",\"ack\":{\"cmd\":\"%s\",\"ok\":false,\"error\":\"%s\"}}",
               NODE_NAME, ack_cmd, ack_err ? ack_err : "unknown-cmd");
    }
  } else {
    snprintf(msg, sizeof(msg),
             "{\"node\":\"%s\",\"kind\":\"state\",\"rgb\":{\"state\":\"%s\",\"hex\":\"%s\",\"r\":%u,\"g\":%u,\"b\":%u}}",
             NODE_NAME, rgb_on ? "on" : "off", rgb_hex, rgb_r, rgb_g, rgb_b);
  }
  mqtt.publish(topic, msg, true);
}

// Apply a hex color ("ff0000" or "#ff0000"). Returns false on malformed hex.
bool rgb_apply_hex(const char* hex) {
  if (hex[0] == '#') hex++;
  if (strlen(hex) != 6) return false;
  for (int i = 0; i < 6; i++) {
    if (!isxdigit((unsigned char)hex[i])) return false;
  }
  rgb_hex_to_parts(hex);
  rgb_on = true;
  apply_rgb();
  return true;
}

// Legacy text protocol: rgb:on | rgb:off | rgb:#RRGGBB (deprecated).
// Returns true if the payload was a legacy rgb command.
bool handle_legacy_rgb(const char* cmd) {
  if (strncmp(cmd, "rgb:", 4) != 0) return false;
  const char* arg = cmd + 4;
  if (strcasecmp(arg, "on") == 0) {
    rgb_on = true;
  } else if (strcasecmp(arg, "off") == 0) {
    rgb_on = false;
  } else if (arg[0] == '#' && strlen(arg) == 7 && rgb_apply_hex(arg)) {
    // applied
  } else {
    publish_state("rgb", false, "bad-legacy-arg");
    return true;
  }
  apply_rgb();
  publish_state("rgb", true);
  return true;
}

void publish_caps() {
  char topic[64];
  snprintf(topic, sizeof(topic), "firment/device/%s/caps", NODE_NAME);
  const char* caps =
      "{\"node\":\"%s\",\"kind\":\"caps\","
      "\"cmds\":[\"ping\",\"rgb.on\",\"rgb.off\",\"rgb.set\"],"
      "\"args\":{\"rgb.set\":[\"hex\"]},"
      "\"note\":\"rgb.set hex=RRGGBB (no #); WS2812@GPIO48\"}";
  char msg[256];
  snprintf(msg, sizeof(msg), caps, NODE_NAME);
  mqtt.publish(topic, msg, true);  // retained: grammar discoverable anytime
}

void handle_cmd(const char* cmd) {
  // Legacy text protocol first (deprecated but accepted).
  if (handle_legacy_rgb(cmd)) return;

  // JSON envelope: {"cmd":"...","args":{...}}
  JsonDocument doc;
  DeserializationError err = deserializeJson(doc, cmd);
  if (err) {
    // Not JSON at all — ack unknown so commands are never silent.
    publish_state(cmd, false, "unknown-cmd");
    return;
  }
  const char* name = doc["cmd"] | "";
  JsonObject args = doc["args"].as<JsonObject>();

  if (strcmp(name, "ping") == 0) {
    publish_state("ping", true);
  } else if (strcmp(name, "rgb.on") == 0) {
    rgb_on = true;
    apply_rgb();
    publish_state(name, true);
  } else if (strcmp(name, "rgb.off") == 0) {
    rgb_on = false;
    apply_rgb();
    publish_state(name, true);
  } else if (strcmp(name, "rgb.set") == 0) {
    const char* hex = args["hex"] | "";
    char with_hash[8];
    snprintf(with_hash, sizeof(with_hash), "#%s", hex);
    if (rgb_apply_hex(with_hash)) {
      publish_state(name, true);
    } else {
      publish_state(name, false, "bad-hex");
    }
  } else {
    publish_state(name, false, "unknown-cmd");
  }
}

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
  handle_cmd(cmd);
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
      // Broker down AND wifi down would otherwise loop doomed TCP attempts
      // forever — re-run the wifi join when the link dropped mid-retry.
      if (WiFi.status() != WL_CONNECTED) {
        connect_wifi();
      }
      delay(1500);
    }
  }
}

void setup() {
  Serial.begin(115200);
  delay(200);
  Serial.println("\n[node] boot");
  neopixelWrite(RGB_PIN, 0, 0, 2);  // dim blue = booting
  connect_wifi();
  mqtt.setServer(MQTT_HOST, MQTT_PORT);
  mqtt.setCallback(on_message);
  connect_mqtt();
  apply_rgb();
  publish_caps();   // retained: advertise the command grammar
  publish_state();  // retained: announce current rgb on (re)connect
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
