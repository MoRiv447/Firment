// ESP32-C3/S3 SuperMini — MQTT telemetry node (P0-4 of docs/sbc-agent.md §5)
//
// Publishes a heartbeat + a fake sensor metric to the SBC broker every 5 s,
// subscribes to its command topic and echoes payloads back as telemetry.
//
// RGB experiment (WS2812 onboard LED):
//   S3 SuperMini: GPIO48. C3 SuperMini boards typically wire it to GPIO8.
//   publish to firment/device/<node>/cmd :
//     rgb:on          -> last color at full brightness
//     rgb:off         -> black
//     rgb:#ff8800     -> set color (hex)
//   Every change is republished RETAINED on .../<node>/state as
//   {"rgb":{"state":"on","hex":"#ff8800"}} so the SBC guard / small model /
//   firm always see the CURRENT light state without asking.
//
// Build (Arduino IDE or arduino-cli):
//   board: "ESP32C3 Dev Module"   (S3: "ESP32S3 Dev Module")
//   libs : PubSubClient (Nick O'Leary)
//   note : neopixelWrite() is builtin to arduino-esp32 >= 2.0.9 (no lib needed)
//
// Flash from the SBC itself (after arduino-cli is installed there) or from
// any desktop with the board plugged in:
//   arduino-cli compile --fqbn "esp32:esp32:esp32s3:CDCOnBoot=cdc" docs\examples\esp32c3-mqtt-node
//   arduino-cli upload  -p <PORT> --fqbn "esp32:esp32:esp32s3:CDCOnBoot=cdc" docs\examples\esp32c3-mqtt-node

#include <WiFi.h>
#include <PubSubClient.h>

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

void publish_state() {
  char topic[64];
  snprintf(topic, sizeof(topic), "firment/device/%s/state", NODE_NAME);
  char msg[160];
  snprintf(msg, sizeof(msg),
           "{\"node\":\"%s\",\"kind\":\"state\",\"rgb\":{\"state\":\"%s\",\"hex\":\"%s\",\"r\":%u,\"g\":%u,\"b\":%u}}",
           NODE_NAME, rgb_on ? "on" : "off", rgb_hex, rgb_r, rgb_g, rgb_b);
  mqtt.publish(topic, msg, true);  // retained: late joiners see current state
}

void handle_cmd(const char* cmd) {
  if (strncmp(cmd, "rgb:", 4) == 0) {
    const char* arg = cmd + 4;
    if (strcasecmp(arg, "on") == 0) {
      rgb_on = true;
    } else if (strcasecmp(arg, "off") == 0) {
      rgb_on = false;
    } else if (arg[0] == '#' && strlen(arg) == 7) {
      rgb_hex_to_parts(arg);
      rgb_on = true;
    } else {
      return;  // unknown rgb arg — ignore silently
    }
    apply_rgb();
    publish_state();
    return;
  }
  // Echo with escaping + cap: raw payloads with quotes would otherwise
  // produce invalid JSON frames.
  char safe[96];
  json_escape(cmd, safe, sizeof(safe));
  publish("echo", safe);
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

// Escape `"` and `\` so an arbitrary echoed payload cannot produce invalid
// JSON; cap length to keep the publish buffer headroom for long node names.
void json_escape(const char* in, char* out, size_t outsz) {
  size_t o = 0;
  for (size_t i = 0; in[i] != '\0' && o + 2 < outsz; i++) {
    if (in[i] == '"' || in[i] == '\\') {
      out[o++] = '\\';
    }
    out[o++] = in[i];
  }
  out[o] = '\0';
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
