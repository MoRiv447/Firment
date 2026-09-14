import type { DeviceTraffic } from './useDeviceTraffic';
import { Callout, Card, Chip } from '../../ui';
import styles from './TrafficPane.module.css';

/**
 * Every node the broker is hearing from, whether or not this project owns it.
 *
 * It is the machine's view, not the project's, which is why it sits above the
 * project picker and collapsed by default: it is where you check that a node is
 * publishing at all before you look for it in the project's own Devices card.
 *
 * The badge has three states because the link has three. `unknown` -- no frame
 * yet, which is what a view opened before the first heartbeat knows -- is not
 * "off", and rendering it as one would report a broken broker that is fine.
 *
 * A device row's timestamp is a clock time rather than `formatStamp`: the counter
 * is `Date.now()` from this window, so every row is today and only the seconds
 * differ between them.
 */
export function TrafficPane({ traffic }: { traffic: DeviceTraffic }) {
  const { devices, alerts, link } = traffic;
  const badge =
    link.state === 'on' ? '● broker online' : link.state === 'off' ? '○ broker off' : '… broker ?';

  return (
    <details className={styles.traffic}>
      <summary className={styles.summary}>All device traffic (unfiltered)</summary>
      <Card
        title="Devices & guard"
        extra={
          <Chip status={link.state === 'on' ? 'ok' : 'neutral'}>{badge}</Chip>
        }
      >
        {devices.length === 0 && alerts.length === 0 ? (
          <p className={styles.none}>
            No device traffic yet. Configure [mqtt] broker in config.toml (e.g.
            "192.168.1.6:1883") and restart; nodes publish to firment/device/#.
          </p>
        ) : (
          <>
            {devices.map((d) => (
              <div key={d.node} className={styles.row}>
                <Chip size="sm" status="running">
                  {d.node}
                </Chip>
                <Chip size="sm">{d.lastKind}</Chip>
                <span className={styles.frame} title={d.lastFrame}>
                  {d.lastFrame}
                </span>
                <span className={styles.count}>
                  ×{d.count} · {new Date(d.ts).toLocaleTimeString()}
                </span>
              </div>
            ))}
            {alerts.length > 0 && (
              <div className={styles.alerts}>
                <p className={styles.alertHead}>Recent alerts ({alerts.length})</p>
                {alerts.slice(0, 5).map((a, i) => (
                  <div key={`${a.ts}-${i}`} className={styles.alert}>
                    <Chip size="sm" status="failed">
                      {a.node}
                    </Chip>
                    <span className={styles.alertFrame}>{a.frame}</span>
                  </div>
                ))}
              </div>
            )}
            {link.error ? (
              <Callout tone="warn">{`mqtt link: ${link.error} (retrying every 3s)`}</Callout>
            ) : null}
          </>
        )}
      </Card>
    </details>
  );
}
