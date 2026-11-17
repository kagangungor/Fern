import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ChevronLeft, Lock, X, Bluetooth, Check, Speaker, Volume2, VolumeX } from "lucide-react";
import { useT } from "../i18n";

interface WifiNetwork {
  ssid: string;
  signal: number;
  secured: boolean;
  connected: boolean;
  known: boolean;
}

interface BluetoothDeviceInfo {
  id: string;
  name: string;
  connected: boolean;
  battery?: number | null;
  can_connect?: boolean;
}

interface BtNearbyDevice {
  id: string;
  name: string;
  kind: string;
}

interface BtPairingRequest {
  id: string;
  kind: "confirm" | "display" | "provide";
  pin?: string | null;
}

interface OutputDevice {
  id: string;
  name: string;
  is_default: boolean;
}

interface Props {
  kind: "wifi" | "bluetooth" | "audio";
  onBack: () => void;
  onOpenWindowsSettings: () => void;
  onContentHeight?: (height: number) => void;
  tip?: (text: string | undefined) => Record<string, unknown>;
}

const REFRESH_MS = 6000;

function SignalBars({ signal }: { signal: number }) {
  const t = useT();
  const level = signal >= 75 ? 4 : signal >= 50 ? 3 : signal >= 25 ? 2 : 1;
  return (
    <span className="cv-signal" aria-label={t("connections.signal", { percent: signal })}>
      {[1, 2, 3, 4].map((bar) => (
        <i key={bar} className={bar <= level ? "on" : ""} style={{ height: 3 + bar * 2.5 }} />
      ))}
    </span>
  );
}


interface AudioSession {
  pid: number;
  name: string;
  volume: number;
  is_muted: boolean;
}

function AppVolumes({ tip }: { tip?: (text: string | undefined) => Record<string, unknown> }) {
  const t = useT();
  const [sessions, setSessions] = useState<AudioSession[] | null>(null);
  const dragging = useRef(false);
  const lastNonZeroVolRef = useRef<Map<number, number>>(new Map());

  useEffect(() => {
    const load = () => {
      if (dragging.current) return;
      invoke<AudioSession[]>("get_audio_sessions").then((list) => {
        setSessions(list);
        list.forEach((s) => {
          if (s.volume > 0) {
            lastNonZeroVolRef.current.set(s.pid, s.volume);
          }
        });
      }).catch(() => setSessions((prev) => prev ?? []));
    };
    load();
    const timer = setInterval(load, 2000);
    return () => clearInterval(timer);
  }, []);

  const setVolume = (pid: number, volume: number) => {
    if (volume > 0) {
      lastNonZeroVolRef.current.set(pid, volume);
    }
    setSessions((prev) => prev?.map((s) => (s.pid === pid ? { ...s, volume, is_muted: volume === 0 } : s)) ?? prev);
    invoke("set_app_volume", { pid, volume }).catch(() => {});
  };

  const toggleMute = (pid: number, currentlyMuted: boolean, vol: number) => {
    if (currentlyMuted) {
      const prev = lastNonZeroVolRef.current.get(pid);
      const nextVol = prev && prev > 0 ? prev : 1.0;
      setVolume(pid, nextVol);
    } else {
      if (vol > 0) {
        lastNonZeroVolRef.current.set(pid, vol);
      }
      setVolume(pid, 0);
    }
  };

  const handleSliderWheel = (e: React.WheelEvent, pid: number, vol: number) => {
    e.stopPropagation();
    const delta = e.deltaY < 0 ? 0.05 : -0.05;
    const next = Math.min(1, Math.max(0, Math.round((vol + delta) * 100) / 100));
    setVolume(pid, next);
  };

  return (
    <div className="cv-section-group">
      <div className="cv-section">{t("connections.audio.apps")}</div>
      {sessions?.length === 0 && <div className="cv-hint">{t("connections.audio.emptyApps")}</div>}
      {sessions?.map((s) => {
        const effectiveVol = s.is_muted ? 0 : s.volume;
        const percent = Math.round(effectiveVol * 100);
        return (
          <div
            key={s.pid}
            className="cv-row cv-app-row"
          >
            <button
              type="button"
              className="cv-app-mute-btn"
              onClick={(e) => {
                e.stopPropagation();
                toggleMute(s.pid, s.is_muted || s.volume === 0, s.volume);
              }}
              {...(tip ? tip(effectiveVol === 0 ? t("overlay.unmute") : t("overlay.mute")) : { title: effectiveVol === 0 ? t("overlay.unmute") : t("overlay.mute") })}
              aria-label={effectiveVol === 0 ? t("overlay.unmute") : t("overlay.mute")}
            >
              {effectiveVol === 0 ? <VolumeX size={13} /> : <Volume2 size={13} />}
            </button>
            <span className="cv-name" title={s.name}>{s.name}</span>
            <div className="cv-slider-track" onWheel={(e) => { e.stopPropagation(); handleSliderWheel(e, s.pid, effectiveVol); }}>
              <input
                type="range"
                min="0"
                max="1"
                step="0.01"
                value={effectiveVol}
                className="cv-slider-input"
                onChange={(e) => setVolume(s.pid, parseFloat(e.target.value))}
                onPointerDown={(e) => { e.stopPropagation(); dragging.current = true; }}
                onPointerUp={() => { dragging.current = false; }}
                onPointerCancel={() => { dragging.current = false; }}
                onClick={(e) => e.stopPropagation()}
              />
              <div
                className={`cv-slider-fill${effectiveVol === 0 ? " muted" : ""}`}
                style={{ width: `${effectiveVol * 100}%` }}
              />
            </div>
            <span
              className="cv-slider-percent"
              style={{ cursor: "pointer" }}
              onClick={(e) => {
                e.stopPropagation();
                toggleMute(s.pid, effectiveVol === 0, effectiveVol);
              }}
              aria-label={t("cc.volume")}
            >
              {percent}%
            </span>
          </div>
        );
      })}
    </div>
  );
}

export function ConnectionsView({ kind, onBack, onOpenWindowsSettings, onContentHeight, tip = () => ({}) }: Props) {
  const headerRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  void tip;
  const [networks, setNetworks] = useState<WifiNetwork[] | null>(null);
  const [devices, setDevices] = useState<BluetoothDeviceInfo[] | null>(null);
  const [outputs, setOutputs] = useState<OutputDevice[] | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState("");
  const [passwordFor, setPasswordFor] = useState<string | null>(null);
  const [password, setPassword] = useState("");
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const [btScanning, setBtScanning] = useState(false);
  const [nearbyDevices, setNearbyDevices] = useState<BtNearbyDevice[]>([]);
  const [pairingRequest, setPairingRequest] = useState<BtPairingRequest | null>(null);
  const [pairingPin, setPairingPin] = useState("");

  useEffect(() => {
    if (passwordFor === null && pairingRequest?.kind !== "provide") return;
    invoke("set_window_keyboard_focus", { enabled: true }).catch(() => {});
    return () => {
      invoke("set_window_keyboard_focus", { enabled: false }).catch(() => {});
    };
  }, [passwordFor, pairingRequest?.kind]);

  const refresh = useCallback(async () => {
    try {
      if (kind === "wifi") setNetworks(await invoke<WifiNetwork[]>("wifi_scan"));
      else if (kind === "audio") setOutputs(await invoke<OutputDevice[]>("get_audio_devices"));
      else setDevices(await invoke<BluetoothDeviceInfo[]>("bluetooth_devices"));
      setError("");
    } catch (e) {
      setError(String(e));
      if (kind === "wifi") setNetworks((prev) => prev ?? []);
      else if (kind === "audio") setOutputs((prev) => prev ?? []);
      else setDevices((prev) => prev ?? []);
    }
  }, [kind]);

  useEffect(() => {
    if (!onContentHeight) return;
    const measure = () => {
      const headerH = headerRef.current?.offsetHeight ?? 24;
      const listH = listRef.current?.scrollHeight ?? 40;
      const errH = error ? 24 : 0;
      // 36px main-row + 24px container vertical padding + 6px flex gap
      onContentHeight(Math.round(66 + headerH + listH + errH));
    };
    const frame = requestAnimationFrame(measure);
    return () => cancelAnimationFrame(frame);
  }, [onContentHeight, kind, networks, devices, outputs, nearbyDevices, btScanning, pairingRequest, passwordFor, confirmRemove, error]);

  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), REFRESH_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  useEffect(() => {
    if (kind !== "bluetooth") return;
    const unlistenNearby = listen<BtNearbyDevice[]>("bluetooth-nearby", (event) => {
      setNearbyDevices(event.payload ?? []);
    });
    const unlistenChanged = listen("bluetooth-changed", () => {
      void refresh();
    });
    const unlistenPairing = listen<BtPairingRequest>("bluetooth-pairing-request", (event) => {
      setPairingRequest(event.payload);
      setPairingPin("");
    });
    return () => {
      unlistenNearby.then((fn) => fn());
      unlistenChanged.then((fn) => fn());
      unlistenPairing.then((fn) => fn());
      invoke("bluetooth_discovery", { active: false }).catch(() => {});
    };
  }, [kind, refresh]);

  const toggleBtScan = () => {
    const next = !btScanning;
    setBtScanning(next);
    if (!next) setNearbyDevices([]);
    invoke("bluetooth_discovery", { active: next }).catch((e) => {
      setError(String(e));
      setBtScanning(false);
    });
  };

  const respondPairing = (accept: boolean) => {
    const pin = pairingRequest?.kind === "provide" ? pairingPin : null;
    setPairingRequest(null);
    setPairingPin("");
    invoke("bluetooth_pairing_respond", { accept, pin }).catch(() => {});
  };

  const t = useT();
  const run = async (label: string, action: () => Promise<unknown>) => {
    setBusy(label);
    setError("");
    try {
      await action();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy("");
      setTimeout(() => void refresh(), 1200);
    }
  };

  const joinNetwork = (network: WifiNetwork) => {
    if (network.connected) return;
    if (network.secured && !network.known) {
      setPasswordFor(network.ssid);
      setPassword("");
      return;
    }
    void run(network.ssid, () => invoke("wifi_connect", { ssid: network.ssid, password: null }));
  };

  const submitPassword = (e: React.FormEvent) => {
    e.preventDefault();
    if (!passwordFor || !password) return;
    const ssid = passwordFor;
    const key = password;
    setPasswordFor(null);
    setPassword("");
    void run(ssid, () => invoke("wifi_connect", { ssid, password: key }));
  };

  const list = kind === "wifi" ? networks : kind === "audio" ? outputs : devices;
  const loading = list === null;
  const empty = list?.length === 0;
  const title = t(kind === "wifi" ? "connections.wifi.title" : kind === "audio" ? "connections.audio.title" : "connections.bluetooth.title");
  const emptyText = t(kind === "wifi" ? "connections.wifi.empty" : kind === "audio" ? "connections.audio.empty" : "connections.bluetooth.empty");

  return (
    <div className="cv-root" onClick={(e) => e.stopPropagation()} onWheel={(e) => e.stopPropagation()}>
      <div ref={headerRef} className="cv-header">
        <button className="cv-back" onClick={onBack} aria-label={t("common.back")}>
          <ChevronLeft size={16} />
        </button>
        <span className="cv-title">{title}</span>
        {kind === "bluetooth" && (
          <button className="cv-link" onClick={toggleBtScan}>
            {btScanning ? t("connections.bluetooth.stopScan") : t("connections.bluetooth.scan")}
          </button>
        )}
        <button className="cv-link" onClick={onOpenWindowsSettings}>{t("connections.windowsSettings")}</button>
      </div>

      <div ref={listRef} className="cv-list" onWheel={(e) => e.stopPropagation()}>
        {loading && <div className="cv-hint">{kind === "wifi" ? t("connections.scanning") : t("connections.loading")}</div>}
        {!loading && empty && !error && <div className="cv-hint">{emptyText}</div>}

        {kind === "audio" && <AppVolumes tip={tip} />}

        {kind === "audio" && outputs && outputs.length > 0 && (
          <div className="cv-section-group">
            <div className="cv-divider" />
            <div className="cv-section">{t("connections.audio.outputs")}</div>
            {outputs.map((o) => (
              <div
                key={o.id}
                className={`cv-row${o.is_default ? " connected" : ""}${busy === o.id ? " busy" : ""}`}
                onClick={() => { if (!o.is_default) void run(o.id, () => invoke("set_default_audio_device", { id: o.id })); }}
              >
                <Speaker size={13} className="cv-bt-icon" />
                <span className="cv-name">{o.name}</span>
                {o.is_default ? <Check size={14} /> : <span className="cv-status">{busy === o.id ? t("connections.switching") : ""}</span>}
              </div>
            ))}
          </div>
        )}

        {kind === "wifi" && networks?.map((n) => (
          <div key={n.ssid}>
            <div
              className={`cv-row${n.connected ? " connected" : ""}${busy === n.ssid ? " busy" : ""}`}
              onClick={() => joinNetwork(n)}
            >
              <SignalBars signal={n.signal} />
              <span className="cv-name">{n.ssid}</span>
              {n.secured && <Lock size={11} className="cv-lock" />}
              <span className="cv-status">{busy === n.ssid ? t("connections.connecting") : n.connected ? t("connections.connected") : n.known ? t("connections.saved") : ""}</span>
              {n.connected && (
                <button className="cv-action" onClick={(e) => { e.stopPropagation(); void run(n.ssid, () => invoke("wifi_disconnect")); }}>
                  {t("connections.disconnect")}
                </button>
              )}
              {n.known && !n.connected && (
                <button
                  className="cv-icon-btn"
                  title={t("connections.forget")}
                  onClick={(e) => { e.stopPropagation(); void run(n.ssid, () => invoke("wifi_forget", { ssid: n.ssid })); }}
                >
                  <X size={12} />
                </button>
              )}
            </div>
            {passwordFor === n.ssid && (
              <form className="cv-password" onSubmit={submitPassword}>
                <input
                  type="password"
                  autoFocus
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder={t("connections.password")}
                  spellCheck={false}
                />
                <button type="submit" disabled={!password}>{t("connections.join")}</button>
              </form>
            )}
          </div>
        ))}

        {kind === "bluetooth" && devices?.map((d) => (
          <div
            key={d.id}
            className={`cv-row${d.connected ? " connected" : ""}${busy === d.id ? " busy" : ""}`}
            onClick={() => {
              if (confirmRemove === d.id) return;
              void run(d.id, () => invoke("bluetooth_set_connected", { id: d.id, connected: !d.connected }));
            }}
          >
            <Bluetooth size={13} className="cv-bt-icon" />
            <span className="cv-name">{d.name}</span>
            <span className="cv-status">
              {busy === d.id
                ? (d.connected ? t("connections.disconnect") : t("connections.connecting"))
                : d.connected
                  ? (d.battery != null ? `${t("connections.connected")} · ${d.battery}%` : t("connections.connected"))
                  : t("connections.paired")}
            </span>
            {confirmRemove === d.id ? (
              <button
                className="cv-action danger"
                onClick={(e) => { e.stopPropagation(); setConfirmRemove(null); void run(d.id, () => invoke("bluetooth_unpair", { id: d.id })); }}
              >
                {t("connections.removeConfirm")}
              </button>
            ) : (
              <button className="cv-icon-btn" title={t("connections.removeDevice")} onClick={(e) => { e.stopPropagation(); setConfirmRemove(d.id); }}>
                <X size={12} />
              </button>
            )}
          </div>
        ))}

        {kind === "bluetooth" && (btScanning || nearbyDevices.length > 0) && (
          <div className="cv-section-group">
            <div className="cv-divider" />
            <div className="cv-section">{t("connections.bluetooth.nearby")}</div>
            {btScanning && nearbyDevices.length === 0 && (
              <div className="cv-hint">{t("connections.scanning")}</div>
            )}
            {nearbyDevices.map((n) => (
              <div key={n.id}>
                <div
                  className={`cv-row${busy === n.id ? " busy" : ""}`}
                  onClick={() => void run(n.id, async () => {
                    try {
                      await invoke("bluetooth_pair", { id: n.id });
                    } finally {
                      setPairingRequest((prev) => (prev?.id === n.id ? null : prev));
                    }
                  })}
                >
                  <Bluetooth size={13} className="cv-bt-icon" />
                  <span className="cv-name">{n.name}</span>
                  <span className="cv-status">
                    {busy === n.id ? t("connections.bluetooth.pairing") : t("connections.bluetooth.pair")}
                  </span>
                </div>
                {pairingRequest?.id === n.id && (
                  <div className="cv-password" onClick={(e) => e.stopPropagation()}>
                    {pairingRequest.kind === "confirm" && (
                      <>
                        <span className="cv-status">{t("connections.bluetooth.confirmPin", { pin: pairingRequest.pin ?? "" })}</span>
                        <button type="button" onClick={() => respondPairing(true)}>{t("connections.bluetooth.accept")}</button>
                        <button type="button" onClick={() => respondPairing(false)}>{t("connections.bluetooth.cancel")}</button>
                      </>
                    )}
                    {pairingRequest.kind === "display" && (
                      <span className="cv-status">{t("connections.bluetooth.enterPinOnDevice", { pin: pairingRequest.pin ?? "" })}</span>
                    )}
                    {pairingRequest.kind === "provide" && (
                      <form onSubmit={(e) => { e.preventDefault(); respondPairing(true); }} style={{ display: "flex", gap: 6, flex: 1 }}>
                        <input
                          type="text"
                          autoFocus
                          value={pairingPin}
                          onChange={(e) => setPairingPin(e.target.value)}
                          placeholder={t("connections.bluetooth.pinPlaceholder")}
                          spellCheck={false}
                        />
                        <button type="submit" disabled={!pairingPin}>{t("connections.bluetooth.accept")}</button>
                        <button type="button" onClick={() => respondPairing(false)}>{t("connections.bluetooth.cancel")}</button>
                      </form>
                    )}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {error && <div className="cv-error">{error}</div>}
    </div>
  );
}

